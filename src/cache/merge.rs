//! Merging a refresh into what den already shows, by source.
//!
//! A fact from the refresh always wins. An earlier fact the refresh did not
//! return is dropped only when the refresh read its source completely (then it
//! is really gone); otherwise it is kept, and the source is marked as carried
//! with the age of the kept data. Coverage is always the refresh's, so a kept
//! source still reads as failed or partial: den never presents old data as new.
//!
//! GitHub Owner and Repo names are case-insensitive, and a fact's Repo carries
//! the Owner as configured. So sources and facts are matched by their folded
//! identity: a config that changes only an Owner's casing still replaces the
//! earlier survey's facts instead of keeping or doubling them.

use super::{CachedObservation, Carried};
use crate::domain::{
    Coverage, Observation, RepoId, Source, SourceKind, SourceState, ids::same_login,
};
use jiff::Timestamp;
use std::{collections::HashSet, hash::Hash};

/// Merges `fresh` into `previous`. A previous Observation of another viewer is
/// never merged: the cache is per viewer.
pub fn merge(previous: Option<CachedObservation>, fresh: Observation) -> CachedObservation {
    let Some(previous) = previous.filter(|p| p.observation.viewer.is(&fresh.viewer.login)) else {
        return CachedObservation::fresh(fresh);
    };
    let mut merged = fresh;
    // Borrows the refresh's coverage; the merge below only moves other fields.
    let reading = Reading::of(merged.owners.clone(), &merged.coverage);
    let carried = carried_sources(&previous, &merged, &reading);
    let previous_observation = previous.observation;

    let mut repos = std::mem::take(&mut merged.repos);
    keep_unread(
        &reading,
        &mut repos,
        previous_observation.repos,
        SourceKind::Repos,
        |r| &mut r.id,
        |r| r.id.folded(),
    );
    let mut pull_requests = std::mem::take(&mut merged.pull_requests);
    keep_unread(
        &reading,
        &mut pull_requests,
        previous_observation.pull_requests,
        SourceKind::PullRequests,
        |i| &mut i.repo,
        |i| i.id().folded(),
    );
    let mut issues = std::mem::take(&mut merged.issues);
    keep_unread(
        &reading,
        &mut issues,
        previous_observation.issues,
        SourceKind::Issues,
        |i| &mut i.repo,
        |i| i.id().folded(),
    );
    let mut branches = std::mem::take(&mut merged.branches);
    keep_unread(
        &reading,
        &mut branches,
        previous_observation.branches,
        SourceKind::Branches,
        |i| &mut i.repo,
        |i| i.id().folded(),
    );
    // A workflow has many Runs; each Run attempt is its own fact.
    let mut runs = std::mem::take(&mut merged.runs);
    keep_unread(
        &reading,
        &mut runs,
        previous_observation.runs,
        SourceKind::Runs,
        |i| &mut i.repo,
        |i| (i.repo.folded(), i.run_id, i.attempt),
    );
    merged.repos = repos;
    merged.pull_requests = pull_requests;
    merged.issues = issues;
    merged.branches = branches;
    merged.runs = runs;
    // `merged.linear` is the refresh's own: nothing fills it before #32, and
    // #34 merges it per Linear source like the GitHub kinds above. Until then
    // no Linear source is ever carried (`carried_sources`), so the header
    // never says Linear data was kept.
    if !reading.viewer_read {
        for team in previous_observation.viewer.teams {
            if !merged.viewer.belongs_to(&team) {
                merged.viewer.teams.push(team);
            }
        }
    }
    CachedObservation {
        observation: merged,
        carried,
    }
}

/// What the refresh read completely.
struct Reading<'f> {
    owners: Vec<String>,
    /// The refresh's coverage.
    coverage: &'f Coverage,
    viewer_read: bool,
}

impl<'f> Reading<'f> {
    /// What a refresh of `owners` that recorded `coverage` read.
    fn of(owners: Vec<String>, coverage: &'f Coverage) -> Self {
        Self {
            owners,
            coverage,
            viewer_read: coverage.get(&Source::Viewer) == Some(&SourceState::Complete),
        }
    }

    /// The refresh's spelling of `owner`, when it still watches that Owner.
    fn owner_as_watched(&self, owner: &str) -> Option<&str> {
        self.owners
            .iter()
            .find(|o| same_login(o, owner))
            .map(String::as_str)
    }

    /// Should an earlier fact of this Repo and kind, absent from the refresh, be
    /// kept? Yes unless the refresh read its source completely, or no longer
    /// watches its Owner.
    ///
    /// "Read completely" is [`Coverage::read_completely`]: a Repo the refresh
    /// recorded a source for is judged by that source alone, any other by its
    /// Owner's. The Landing Groups' "merged or closed" uses the same rule.
    fn keeps(&self, repo: &RepoId, kind: SourceKind) -> bool {
        if !self.owners.iter().any(|o| same_login(o, &repo.owner)) {
            return false;
        }
        !self.coverage.read_completely(repo, kind)
    }
}

/// Appends to `fresh` the `previous` facts it lacks whose source the refresh
/// could not read completely.
fn keep_unread<T, K: Eq + Hash>(
    reading: &Reading<'_>,
    fresh: &mut Vec<T>,
    previous: Vec<T>,
    kind: SourceKind,
    repo_of: impl Fn(&mut T) -> &mut RepoId,
    key_of: impl Fn(&T) -> K,
) {
    let seen: HashSet<K> = fresh.iter().map(&key_of).collect();
    let kept: Vec<T> = previous
        .into_iter()
        .filter_map(|mut item| {
            if seen.contains(&key_of(&item)) {
                return None;
            }
            let repo = repo_of(&mut item);
            if !reading.keeps(repo, kind) {
                return None;
            }
            // A kept fact takes the Owner's casing from the refresh, so it is
            // the same Repo as the refresh's facts wherever den groups by Repo
            // (a newer Run of its workflow supersedes it, the sidebar shows one
            // Repo).
            if let Some(owner) = reading.owner_as_watched(&repo.owner) {
                owner.clone_into(&mut repo.owner);
            }
            Some(item)
        })
        .collect();
    fresh.extend(kept);
}

/// For every source the refresh did not read completely, how old the data den
/// still holds for it is (if it holds any).
///
/// A Repo whose earlier data was already older than its Owner's keeps its own age
/// while its facts are still kept, even when the refresh recorded nothing for that
/// Repo (its Owner's query failed before reaching it). Otherwise it would fall
/// back to the Owner's age and read as newer than it is.
///
/// Linear sources are never carried: [`merge`] keeps no earlier Linear data
/// (#34 adds that), so a failed Linear source has nothing kept to date.
fn carried_sources(
    previous: &CachedObservation,
    fresh: &Observation,
    reading: &Reading<'_>,
) -> Vec<Carried> {
    let mut carried: Vec<Carried> = fresh
        .coverage
        .incomplete()
        .filter(|entry| !entry.source.is_linear())
        .filter_map(|entry| {
            data_age(previous, &entry.source).map(|observed_at| Carried {
                source: entry.source.clone(),
                observed_at,
            })
        })
        .collect();
    for earlier in &previous.carried {
        let Source::Repo { repo, kind } = &earlier.source else {
            continue;
        };
        let folded = fold_source(&earlier.source);
        let listed = carried.iter().any(|c| fold_source(&c.source) == folded);
        if !listed && reading.keeps(repo, *kind) {
            carried.push(earlier.clone());
        }
    }
    carried
}

/// When the data `previous` holds for `source` was observed; `None` when it holds
/// none (the source failed or was never fetched).
fn data_age(previous: &CachedObservation, source: &Source) -> Option<Timestamp> {
    let folded = fold_source(source);
    if let Some(carried) = previous
        .carried
        .iter()
        .find(|c| fold_source(&c.source) == folded)
    {
        return Some(carried.observed_at);
    }
    let state = previous
        .observation
        .coverage
        .entries()
        .iter()
        .find(|e| fold_source(&e.source) == folded)
        .map(|e| &e.state);
    match state {
        Some(SourceState::Complete | SourceState::Partial(_)) => {
            Some(previous.observation.observed_at)
        }
        Some(SourceState::Failed { .. } | SourceState::NotYetFetched) => None,
        None => match source {
            Source::Repo { repo, kind } => data_age(
                previous,
                &Source::Owner {
                    owner: repo.owner.clone(),
                    kind: *kind,
                },
            ),
            Source::Owner { .. } | Source::Viewer => None,
        },
    }
}

/// A source's case-insensitive identity.
fn fold_source(source: &Source) -> Source {
    match source {
        Source::Viewer => Source::Viewer,
        Source::Owner { owner, kind } => Source::Owner {
            owner: owner.to_ascii_lowercase(),
            kind: *kind,
        },
        Source::Repo { repo, kind } => Source::Repo {
            repo: repo.folded(),
            kind: *kind,
        },
    }
}
