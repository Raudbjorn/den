//! Merging a refresh into what den already shows, by source.
//!
//! A fact from the refresh always wins. An earlier fact the refresh did not
//! return is dropped only when the refresh read its source completely (then it
//! is really gone); otherwise it is kept, and the source is marked as carried
//! with the age of the kept data. Coverage is always the refresh's, so a kept
//! source still reads as failed or partial: den never presents old data as new.
//!
//! GitHub and Linear are surveyed apart, so each refresh speaks for its own
//! sources only. [`merge`] takes a GitHub survey and leaves the Linear part, its
//! coverage and its ages as they were; [`merge_linear`] takes a Linear survey
//! and leaves GitHub's alone. Both apply the one rule above ([`Reading`],
//! [`keep_unread`]): a fact names its source, a GitHub fact by its kind and
//! Repo, a Linear fact by its Owner and the Linear source that listed it. A
//! Linear fact list added later (a start team's states, say) is one more
//! [`keep_unread`] call in [`merge_linear`].
//!
//! Linear data is kept only for the Linear viewer it was read as. When a Linear
//! refresh reads another Linear viewer for an Owner (another account behind the
//! key, or another workspace), all of that Owner's earlier Linear data is
//! dropped, whatever its sources' coverage says.
//!
//! GitHub Owner and Repo names are case-insensitive, and a fact's Repo carries
//! the Owner as configured. So sources and facts are matched by their folded
//! identity: a config that changes only an Owner's casing still replaces the
//! earlier survey's facts instead of keeping or doubling them.

use super::{CachedObservation, Carried, source_owner};
use crate::{
    domain::{
        Coverage, LinearPart, LinearSourceKind, Observation, RepoId, Source, SourceKind,
        SourceState, ids::same_login,
    },
    ports::LinearSurvey,
};
use jiff::Timestamp;
use std::{collections::HashSet, hash::Hash};

/// Merges a GitHub survey `fresh` into `previous`. A previous Observation of
/// another viewer is never merged: the cache is per viewer. Whatever Linear
/// part or Linear coverage `fresh` carries is ignored: a GitHub survey says
/// nothing about Linear.
pub fn merge(previous: Option<CachedObservation>, mut fresh: Observation) -> CachedObservation {
    fresh.linear = LinearPart::default();
    fresh.coverage.retain(|e| !e.source.is_linear());
    let Some(previous) = previous.filter(|p| p.observation.viewer.is(&fresh.viewer.login)) else {
        return CachedObservation::fresh(fresh);
    };
    let reading = Reading::of(fresh.owners.clone(), &fresh.coverage);
    let mut carried = carried_sources(&previous, &fresh, &reading);
    carried.extend(
        previous
            .carried
            .iter()
            .filter(|c| c.source.is_linear())
            .cloned(),
    );
    let mut merged = fresh;
    let previous_observation = previous.observation;

    let mut repos = std::mem::take(&mut merged.repos);
    keep_unread(
        &reading,
        &mut repos,
        previous_observation.repos,
        |reading, r| reading.keeps_repo_fact(&r.id, SourceKind::Repos),
        |r| &mut r.id.owner,
        |r| r.id.folded(),
    );
    let mut pull_requests = std::mem::take(&mut merged.pull_requests);
    keep_unread(
        &reading,
        &mut pull_requests,
        previous_observation.pull_requests,
        |reading, i| reading.keeps_repo_fact(&i.repo, SourceKind::PullRequests),
        |i| &mut i.repo.owner,
        |i| i.id().folded(),
    );
    let mut issues = std::mem::take(&mut merged.issues);
    keep_unread(
        &reading,
        &mut issues,
        previous_observation.issues,
        |reading, i| reading.keeps_repo_fact(&i.repo, SourceKind::Issues),
        |i| &mut i.repo.owner,
        |i| i.id().folded(),
    );
    let mut branches = std::mem::take(&mut merged.branches);
    keep_unread(
        &reading,
        &mut branches,
        previous_observation.branches,
        |reading, i| reading.keeps_repo_fact(&i.repo, SourceKind::Branches),
        |i| &mut i.repo.owner,
        |i| i.id().folded(),
    );
    // A workflow has many Runs; each Run attempt is its own fact.
    let mut runs = std::mem::take(&mut merged.runs);
    keep_unread(
        &reading,
        &mut runs,
        previous_observation.runs,
        |reading, i| reading.keeps_repo_fact(&i.repo, SourceKind::Runs),
        |i| &mut i.repo.owner,
        |i| (i.repo.folded(), i.run_id, i.attempt),
    );
    merged.repos = repos;
    merged.pull_requests = pull_requests;
    merged.issues = issues;
    merged.branches = branches;
    merged.runs = runs;
    // Linear stays as the previous Observation had it: its part, its coverage
    // (carried above, its ages).
    merged.linear = previous_observation.linear;
    for entry in previous_observation.coverage.entries() {
        if entry.source.is_linear() {
            merged
                .coverage
                .record(entry.source.clone(), entry.state.clone());
        }
    }
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

/// Merges a Linear survey into `previous`, Linear source by Linear source;
/// GitHub's part, coverage and ages stay as they were.
///
/// The survey speaks for the Owners it records a Linear source for (the App
/// records every configured one, as not yet fetched when the survey said
/// nothing): earlier Linear data of any other Owner is dropped, as it no
/// longer has a Linear Workflow. A Linear source of such an Owner that was
/// recorded before but not now is not yet fetched, so data kept for it never
/// reads as complete.
pub fn merge_linear(previous: CachedObservation, survey: LinearSurvey) -> CachedObservation {
    let LinearSurvey {
        part: fresh,
        coverage: answered,
    } = survey;
    let coverage = linear_coverage(&previous.observation.coverage, &answered);
    let workflow_owners = linear_owners(&coverage);
    let changed = changed_linear_viewers(&previous.observation.linear, &fresh);
    let reread = |owner: &str| changed.iter().any(|o| same_login(o, owner));
    let reading = Reading::of(workflow_owners, &coverage);
    let linear_carried: Vec<Carried> = coverage
        .incomplete()
        .filter(|e| source_owner(&e.source).is_some_and(|o| !reread(o)))
        .filter_map(|e| {
            data_age(&previous, &e.source).map(|observed_at| Carried {
                source: e.source.clone(),
                observed_at,
            })
        })
        .collect();

    let CachedObservation {
        mut observation,
        carried,
    } = previous;
    let mut earlier = std::mem::take(&mut observation.linear);
    earlier.viewers.retain(|v| !reread(&v.owner));
    earlier.issues.retain(|i| !reread(&i.owner));
    earlier.team_states.retain(|t| !reread(&t.owner));
    let mut merged = fresh;
    keep_unread(
        &reading,
        &mut merged.viewers,
        earlier.viewers,
        |reading, v| {
            reading.keeps_owner_fact(&v.owner, SourceKind::Linear(LinearSourceKind::Viewer))
        },
        |v| &mut v.owner,
        |v| v.owner.to_ascii_lowercase(),
    );
    keep_unread(
        &reading,
        &mut merged.issues,
        earlier.issues,
        |reading, i| reading.keeps_owner_fact(&i.owner, SourceKind::Linear(i.listed_by)),
        |i| &mut i.owner,
        |i| i.work_item_id().folded(),
    );
    // #33's team states: one list per Owner, keyed by Linear's team ID. A
    // team-states source that failed on a config mismatch still carries the
    // teams it read, which replace the earlier ones by key.
    keep_unread(
        &reading,
        &mut merged.team_states,
        earlier.team_states,
        |reading, t| {
            reading.keeps_owner_fact(&t.owner, SourceKind::Linear(LinearSourceKind::TeamStates))
        },
        |t| &mut t.owner,
        |t| (t.owner.to_ascii_lowercase(), t.team.id.clone()),
    );
    observation.linear = merged;
    observation.coverage.retain(|e| !e.source.is_linear());
    for entry in coverage.entries() {
        observation
            .coverage
            .record(entry.source.clone(), entry.state.clone());
    }
    let mut carried: Vec<Carried> = carried
        .into_iter()
        .filter(|c| !c.source.is_linear())
        .collect();
    carried.extend(linear_carried);
    CachedObservation {
        observation,
        carried,
    }
}

/// The Owners whose Linear viewer a Linear refresh read as someone else than
/// `previous` holds: another Linear user, or another workspace (also when only
/// an earlier Linear Issue or team's states name the other workspace). Their
/// earlier Linear data
/// was read for another account, so none of it may be kept.
pub fn changed_linear_viewers(previous: &LinearPart, fresh: &LinearPart) -> Vec<String> {
    fresh
        .viewers
        .iter()
        .filter(|now| {
            let other_viewer = previous.viewers.iter().any(|before| {
                same_login(&before.owner, &now.owner)
                    && (before.id != now.id
                        || !before.workspace.eq_ignore_ascii_case(&now.workspace))
            });
            let other_workspace = previous
                .issues
                .iter()
                .map(|issue| (&issue.owner, &issue.id.workspace))
                .chain(
                    previous
                        .team_states
                        .iter()
                        .map(|t| (&t.owner, &t.workspace)),
                )
                .any(|(owner, workspace)| {
                    same_login(owner, &now.owner) && !workspace.eq_ignore_ascii_case(&now.workspace)
                });
            other_viewer || other_workspace
        })
        .map(|v| v.owner.clone())
        .collect()
}

/// The Linear coverage after `answered`: its entries, plus, for each Owner it
/// answered for, any Linear source `previous` recorded and it didn't, as not
/// yet fetched.
fn linear_coverage(previous: &Coverage, answered: &Coverage) -> Coverage {
    let mut coverage = Coverage::default();
    for entry in answered.entries().iter().filter(|e| e.source.is_linear()) {
        coverage.record(entry.source.clone(), entry.state.clone());
    }
    let owners = linear_owners(&coverage);
    for entry in previous.entries().iter().filter(|e| e.source.is_linear()) {
        let answered_for_owner = source_owner(&entry.source)
            .is_some_and(|owner| owners.iter().any(|o| same_login(o, owner)));
        let unanswered = !coverage
            .entries()
            .iter()
            .any(|e| e.source.same(&entry.source));
        if answered_for_owner && unanswered {
            coverage.record(entry.source.clone(), SourceState::NotYetFetched);
        }
    }
    coverage
}

/// The Owners `coverage` records a Linear source for, once each.
fn linear_owners(coverage: &Coverage) -> Vec<String> {
    let mut owners: Vec<String> = Vec::new();
    for entry in coverage.entries().iter().filter(|e| e.source.is_linear()) {
        if let Some(owner) = source_owner(&entry.source)
            && !owners.iter().any(|o| same_login(o, owner))
        {
            owners.push(owner.to_owned());
        }
    }
    owners
}

/// What the refresh read completely.
struct Reading {
    owners: Vec<String>,
    /// The refresh's coverage.
    coverage: Coverage,
    viewer_read: bool,
}

impl Reading {
    /// What a refresh of `owners` that recorded `coverage` read.
    fn of(owners: Vec<String>, coverage: &Coverage) -> Self {
        Self {
            owners,
            coverage: coverage.clone(),
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

    /// Should an earlier GitHub fact of this Repo and kind, absent from the
    /// refresh, be kept? Yes unless the refresh read its source completely, or
    /// no longer watches its Owner.
    ///
    /// "Read completely" is [`Coverage::read_completely`]: a Repo the refresh
    /// recorded a source for is judged by that source alone, any other by its
    /// Owner's. The Landing Groups' "merged or closed" uses the same rule.
    fn keeps_repo_fact(&self, repo: &RepoId, kind: SourceKind) -> bool {
        if !self.owners.iter().any(|o| same_login(o, &repo.owner)) {
            return false;
        }
        !self.coverage.read_completely(repo, kind)
    }

    /// Should an earlier fact read for `owner` by its `kind` source (a Linear
    /// fact), absent from the refresh, be kept? Yes unless the refresh read that
    /// source completely, or no longer reads for that Owner.
    fn keeps_owner_fact(&self, owner: &str, kind: SourceKind) -> bool {
        if self.owner_as_watched(owner).is_none() {
            return false;
        }
        self.owner_source(owner, kind)
            .is_none_or(|state| !state.is_complete())
    }

    /// How the refresh read `owner`'s `kind` source, if it recorded it.
    /// Matched case-insensitively, as the config may spell an Owner either way.
    fn owner_source(&self, owner: &str, kind: SourceKind) -> Option<&SourceState> {
        self.coverage
            .entries()
            .iter()
            .find_map(|e| match &e.source {
                Source::Owner { owner: o, kind: k } if *k == kind && same_login(o, owner) => {
                    Some(&e.state)
                }
                _ => None,
            })
    }
}

/// Appends to `fresh` the `previous` facts it lacks whose source the refresh
/// could not read completely (`keeps` says which, from the fact's own source).
fn keep_unread<T, K: Eq + Hash>(
    reading: &Reading,
    fresh: &mut Vec<T>,
    previous: Vec<T>,
    keeps: impl Fn(&Reading, &T) -> bool,
    owner_of: impl Fn(&mut T) -> &mut String,
    key_of: impl Fn(&T) -> K,
) {
    let seen: HashSet<K> = fresh.iter().map(&key_of).collect();
    let kept: Vec<T> = previous
        .into_iter()
        .filter_map(|mut item| {
            if seen.contains(&key_of(&item)) || !keeps(reading, &item) {
                return None;
            }
            // A kept fact takes the Owner's casing from the refresh, so it is
            // the same Repo (or Owner) as the refresh's facts wherever den
            // groups by them (a newer Run of its workflow supersedes it, the
            // sidebar shows one Repo).
            let owner = owner_of(&mut item);
            if let Some(watched) = reading.owner_as_watched(owner) {
                watched.clone_into(owner);
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
    reading: &Reading,
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
        if !listed && reading.keeps_repo_fact(repo, *kind) {
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
            Some(observed_at(previous, source))
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

/// When `previous` read `source`, unless it carried that source's data over:
/// its Linear survey's time for a Linear source, its GitHub survey's otherwise.
fn observed_at(previous: &CachedObservation, source: &Source) -> Timestamp {
    let observation = &previous.observation;
    match observation.linear.observed_at {
        Some(linear) if source.is_linear() => linear,
        _ => observation.observed_at,
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
