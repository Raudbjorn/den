//! Merging a refresh into what den already shows, by source.
//!
//! A fact from the refresh always wins. An earlier fact the refresh did not
//! return is dropped only when the refresh read its source completely (then it
//! is really gone); otherwise it is kept, and the source is marked as carried
//! with the age of the kept data. Coverage is always the refresh's, so a kept
//! source still reads as failed or partial: den never presents old data as new.

use super::{CachedObservation, Carried};
use crate::domain::{Observation, RepoId, Source, SourceKind, SourceState, ids::same_login};
use jiff::Timestamp;
use std::{collections::HashSet, hash::Hash};

/// Merges `fresh` into `previous`. A previous Observation of another viewer is
/// never merged: the cache is per viewer.
pub fn merge(previous: Option<CachedObservation>, fresh: Observation) -> CachedObservation {
    let Some(previous) = previous.filter(|p| p.observation.viewer.is(&fresh.viewer.login)) else {
        return CachedObservation::fresh(fresh);
    };
    let carried = carried_sources(&previous, &fresh);
    let mut merged = fresh;
    let reading = Reading::of(&merged);
    let previous_observation = previous.observation;

    let mut repos = std::mem::take(&mut merged.repos);
    keep_unread(
        &reading,
        &mut repos,
        previous_observation.repos,
        SourceKind::Repos,
        |r| &r.id,
        |r| r.id.clone(),
    );
    let mut pull_requests = std::mem::take(&mut merged.pull_requests);
    keep_unread(
        &reading,
        &mut pull_requests,
        previous_observation.pull_requests,
        SourceKind::PullRequests,
        |i| &i.repo,
        |i| i.id(),
    );
    let mut issues = std::mem::take(&mut merged.issues);
    keep_unread(
        &reading,
        &mut issues,
        previous_observation.issues,
        SourceKind::Issues,
        |i| &i.repo,
        |i| i.id(),
    );
    let mut branches = std::mem::take(&mut merged.branches);
    keep_unread(
        &reading,
        &mut branches,
        previous_observation.branches,
        SourceKind::Branches,
        |i| &i.repo,
        |i| i.id(),
    );
    // A workflow has many Runs; each Run attempt is its own fact.
    let mut runs = std::mem::take(&mut merged.runs);
    keep_unread(
        &reading,
        &mut runs,
        previous_observation.runs,
        SourceKind::Runs,
        |i| &i.repo,
        |i| (i.repo.clone(), i.run_id, i.attempt),
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
    /// Sources the refresh read completely.
    complete: HashSet<Source>,
    /// Sources the refresh recorded, in any state.
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
                .map(|e| e.source.clone())
                .collect(),
            recorded: entries.iter().map(|e| e.source.clone()).collect(),
            viewer_read: fresh.coverage.get(&Source::Viewer) == Some(&SourceState::Complete),
        }
    }

    /// Should an earlier fact of this Repo and kind, absent from the refresh, be
    /// kept? Yes unless the refresh read its source completely, or no longer
    /// watches its Owner.
    fn keeps(&self, repo: &RepoId, kind: SourceKind) -> bool {
        if !self.owners.iter().any(|o| same_login(o, &repo.owner)) {
            return false;
        }
        let owner_source = Source::Owner {
            owner: repo.owner.clone(),
            kind,
        };
        let repo_source = Source::Repo {
            repo: repo.clone(),
            kind,
        };
        let owner_complete = self.complete.contains(&owner_source);
        let repo_complete =
            !self.recorded.contains(&repo_source) || self.complete.contains(&repo_source);
        !(owner_complete && repo_complete)
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
fn carried_sources(previous: &CachedObservation, fresh: &Observation) -> Vec<Carried> {
    fresh
        .coverage
        .incomplete()
        .filter_map(|entry| {
            data_age(previous, &entry.source).map(|observed_at| Carried {
                source: entry.source.clone(),
                observed_at,
            })
        })
        .collect()
}

/// When the data `previous` holds for `source` was observed; `None` when it holds
/// none (the source failed or was never fetched).
fn data_age(previous: &CachedObservation, source: &Source) -> Option<Timestamp> {
    if let Some(carried) = previous.carried.iter().find(|c| &c.source == source) {
        return Some(carried.observed_at);
    }
    match previous.observation.coverage.get(source) {
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
