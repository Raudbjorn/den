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
    Observation, RepoId, Source, SourceKind, SourceState, WorkItemId, ids::same_login,
};
use jiff::Timestamp;
use std::{collections::HashSet, hash::Hash};

/// Merges `fresh` into `previous`. A previous Observation of another viewer is
/// never merged: the cache is per viewer.
pub fn merge(previous: Option<CachedObservation>, fresh: Observation) -> CachedObservation {
    let Some(previous) = previous.filter(|p| p.observation.viewer.is(&fresh.viewer.login)) else {
        return CachedObservation::fresh(fresh);
    };
    let reading = Reading::of(&fresh);
    let carried = carried_sources(&previous, &fresh, &reading);
    let mut merged = fresh;
    let previous_observation = previous.observation;

    let mut repos = std::mem::take(&mut merged.repos);
    keep_unread(
        &reading,
        &mut repos,
        previous_observation.repos,
        SourceKind::Repos,
        |r| &r.id,
        |r| fold_repo(&r.id),
    );
    let mut pull_requests = std::mem::take(&mut merged.pull_requests);
    keep_unread(
        &reading,
        &mut pull_requests,
        previous_observation.pull_requests,
        SourceKind::PullRequests,
        |i| &i.repo,
        |i| fold_item(i.id()),
    );
    let mut issues = std::mem::take(&mut merged.issues);
    keep_unread(
        &reading,
        &mut issues,
        previous_observation.issues,
        SourceKind::Issues,
        |i| &i.repo,
        |i| fold_item(i.id()),
    );
    let mut branches = std::mem::take(&mut merged.branches);
    keep_unread(
        &reading,
        &mut branches,
        previous_observation.branches,
        SourceKind::Branches,
        |i| &i.repo,
        |i| fold_item(i.id()),
    );
    // A workflow has many Runs; each Run attempt is its own fact.
    let mut runs = std::mem::take(&mut merged.runs);
    keep_unread(
        &reading,
        &mut runs,
        previous_observation.runs,
        SourceKind::Runs,
        |i| &i.repo,
        |i| (fold_repo(&i.repo), i.run_id, i.attempt),
    );
    merged.repos = repos;
    merged.pull_requests = pull_requests;
    merged.issues = issues;
    merged.branches = branches;
    merged.runs = runs;
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
struct Reading {
    owners: Vec<String>,
    /// Sources the refresh read completely (folded).
    complete: HashSet<Source>,
    /// Sources the refresh recorded, in any state (folded).
    recorded: HashSet<Source>,
    viewer_read: bool,
}

impl Reading {
    fn of(fresh: &Observation) -> Self {
        let entries = fresh.coverage.entries();
        Self {
            owners: fresh.owners.clone(),
            complete: entries
                .iter()
                .filter(|e| e.state.is_complete())
                .map(|e| fold_source(&e.source))
                .collect(),
            recorded: entries.iter().map(|e| fold_source(&e.source)).collect(),
            viewer_read: fresh.coverage.get(&Source::Viewer) == Some(&SourceState::Complete),
        }
    }

    /// Should an earlier fact of this Repo and kind, absent from the refresh, be
    /// kept? Yes unless the refresh read its source completely, or no longer
    /// watches its Owner.
    ///
    /// A Repo the refresh recorded a source for is judged by that source alone:
    /// read completely, its facts are all there even if a later page of its
    /// Owner's list was lost. A Repo with no source of its own was read (or not)
    /// as part of its Owner's list, so only a complete Owner says it is all there.
    fn keeps(&self, repo: &RepoId, kind: SourceKind) -> bool {
        if !self.owners.iter().any(|o| same_login(o, &repo.owner)) {
            return false;
        }
        let repo = fold_repo(repo);
        let owner_source = Source::Owner {
            owner: repo.owner.clone(),
            kind,
        };
        let repo_source = Source::Repo { repo, kind };
        let read_completely = if self.recorded.contains(&repo_source) {
            self.complete.contains(&repo_source)
        } else {
            self.complete.contains(&owner_source)
        };
        !read_completely
    }
}

/// Appends to `fresh` the `previous` facts it lacks whose source the refresh
/// could not read completely.
fn keep_unread<T, K: Eq + Hash>(
    reading: &Reading,
    fresh: &mut Vec<T>,
    previous: Vec<T>,
    kind: SourceKind,
    repo_of: impl Fn(&T) -> &RepoId,
    key_of: impl Fn(&T) -> K,
) {
    let seen: HashSet<K> = fresh.iter().map(&key_of).collect();
    let kept: Vec<T> = previous
        .into_iter()
        .filter(|item| !seen.contains(&key_of(item)) && reading.keeps(repo_of(item), kind))
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
fn carried_sources(
    previous: &CachedObservation,
    fresh: &Observation,
    reading: &Reading,
) -> Vec<Carried> {
    let mut carried: Vec<Carried> = fresh
        .coverage
        .incomplete()
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

/// A Repo's case-insensitive identity.
fn fold_repo(repo: &RepoId) -> RepoId {
    RepoId::new(
        repo.owner.to_ascii_lowercase(),
        repo.name.to_ascii_lowercase(),
    )
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
            repo: fold_repo(repo),
            kind: *kind,
        },
    }
}

/// A Work Item's identity with its Repo folded. Branch and workflow names stay
/// as they are: git branch names are case-sensitive.
fn fold_item(id: WorkItemId) -> WorkItemId {
    match id {
        WorkItemId::Issue { repo, number } => WorkItemId::Issue {
            repo: fold_repo(&repo),
            number,
        },
        WorkItemId::PullRequest { repo, number } => WorkItemId::PullRequest {
            repo: fold_repo(&repo),
            number,
        },
        WorkItemId::Branch { repo, name } => WorkItemId::Branch {
            repo: fold_repo(&repo),
            name,
        },
        WorkItemId::Run { repo, workflow } => WorkItemId::Run {
            repo: fold_repo(&repo),
            workflow,
        },
    }
}
