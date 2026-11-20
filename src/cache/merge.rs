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
//! [`keep_unread`] call in [`merge_linear`], and one more `retain` by its
//! source in [`CachedObservation::restricted_to_linear`].
//!
//! A Linear survey speaks for exactly the Linear sources it records (the App
//! records every one its config asks for, as not yet fetched when the survey
//! said nothing). A Linear source it doesn't record is no longer read: its
//! coverage, its age and the facts it listed are dropped, so it never stays
//! "not yet fetched" nor keeps its facts for good. The rule is the one above,
//! by source, so it holds for every Linear source kind.
//!
//! Linear data is kept only for the Linear viewer it was read as. When a Linear
//! refresh reads another Linear viewer for an Owner (another account behind the
//! key, or another workspace), all of that Owner's earlier Linear data is
//! dropped, whatever its sources' coverage says, and [`LinearMerge`] names the
//! Owner so the App can say so.
//!
//! GitHub Owner and Repo names are case-insensitive, and a fact's Repo carries
//! the Owner as configured. So sources and facts are matched by their folded
//! identity: a config that changes only an Owner's casing still replaces the
//! earlier survey's facts instead of keeping or doubling them.

use super::{CachedObservation, Carried, source_owner};
use crate::{
    domain::{
        Coverage, LinearPart, LinearSourceKind, Observation, RepoId, Source, SourceKind,
        SourceState, WorkItemId, ids::same_login,
    },
    ports::LinearSurvey,
};
use jiff::Timestamp;
use std::{collections::HashSet, hash::Hash};

/// A GitHub survey merged into what den showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubMerge {
    pub cached: CachedObservation,
    /// Nothing earlier was merged: there was none, or it was another viewer's
    /// (the cache is per viewer). Its Linear part went with it, so the App
    /// gives the merge this session's Linear answer.
    pub afresh: bool,
}

/// A Linear survey merged into what den showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearMerge {
    pub cached: CachedObservation,
    /// The Owners whose Linear viewer the survey read as someone else than
    /// the earlier Linear data was read as: all of their earlier Linear data
    /// was dropped.
    pub viewer_changed: Vec<String>,
}

/// Merges a GitHub survey `fresh` into `previous`. A previous Observation of
/// another viewer is never merged: the cache is per viewer. Whatever Linear
/// part or Linear coverage `fresh` carries is ignored: a GitHub survey says
/// nothing about Linear.
pub fn merge(previous: Option<CachedObservation>, mut fresh: Observation) -> GitHubMerge {
    fresh.linear = LinearPart::default();
    fresh.coverage.retain(|e| !e.source.is_linear());
    let Some(previous) = previous.filter(|p| p.observation.viewer.is(&fresh.viewer.login)) else {
        return GitHubMerge {
            cached: CachedObservation::fresh(fresh),
            afresh: true,
        };
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
        |r| fold_repo(&r.id),
    );
    let mut pull_requests = std::mem::take(&mut merged.pull_requests);
    keep_unread(
        &reading,
        &mut pull_requests,
        previous_observation.pull_requests,
        |reading, i| reading.keeps_repo_fact(&i.repo, SourceKind::PullRequests),
        |i| &mut i.repo.owner,
        |i| fold_item(i.id()),
    );
    let mut issues = std::mem::take(&mut merged.issues);
    keep_unread(
        &reading,
        &mut issues,
        previous_observation.issues,
        |reading, i| reading.keeps_repo_fact(&i.repo, SourceKind::Issues),
        |i| &mut i.repo.owner,
        |i| fold_item(i.id()),
    );
    let mut branches = std::mem::take(&mut merged.branches);
    keep_unread(
        &reading,
        &mut branches,
        previous_observation.branches,
        |reading, i| reading.keeps_repo_fact(&i.repo, SourceKind::Branches),
        |i| &mut i.repo.owner,
        |i| fold_item(i.id()),
    );
    // A workflow has many Runs; each Run attempt is its own fact.
    let mut runs = std::mem::take(&mut merged.runs);
    keep_unread(
        &reading,
        &mut runs,
        previous_observation.runs,
        |reading, i| reading.keeps_repo_fact(&i.repo, SourceKind::Runs),
        |i| &mut i.repo.owner,
        |i| (fold_repo(&i.repo), i.run_id, i.attempt),
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
    GitHubMerge {
        cached: CachedObservation {
            observation: merged,
            carried,
        },
        afresh: false,
    }
}

/// Merges a Linear survey into `previous`, Linear source by Linear source;
/// GitHub's part, coverage and ages stay as they were.
///
/// The survey speaks for the Linear sources it records, and only those: the
/// App records every one its config asks for (as not yet fetched when the
/// survey said nothing). Earlier Linear data of a source it doesn't record,
/// and so of an Owner it records nothing for, is no longer read and is
/// dropped with that source's coverage and age.
///
/// Borrows the survey: the App keeps it for a GitHub survey that starts
/// afresh. The survey's facts are copied once, into the merged Observation.
pub fn merge_linear(previous: CachedObservation, survey: &LinearSurvey) -> LinearMerge {
    let coverage = linear_coverage(&survey.coverage);
    let workflow_owners = linear_owners(&coverage);
    let viewer_changed = changed_linear_viewers(&previous.observation.linear, &survey.part);
    let reread = |owner: &str| viewer_changed.iter().any(|o| same_login(o, owner));
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
    let mut merged = survey.part.clone();
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
        |i| fold_item(i.work_item_id()),
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
    LinearMerge {
        cached: CachedObservation {
            observation,
            carried,
        },
        viewer_changed,
    }
}

/// The Owners whose Linear viewer a Linear refresh read as someone else than
/// `previous` holds: another Linear user, or another workspace (also when only
/// an earlier Linear Issue names the other workspace). Their earlier Linear data
/// was read for another account, so none of it may be kept.
fn changed_linear_viewers(previous: &LinearPart, fresh: &LinearPart) -> Vec<String> {
    fresh
        .viewers
        .iter()
        .filter(|now| {
            let other_viewer = previous.viewers.iter().any(|before| {
                same_login(&before.owner, &now.owner)
                    && (before.id != now.id
                        || !before.workspace.eq_ignore_ascii_case(&now.workspace))
            });
            let other_workspace = previous.issues.iter().any(|issue| {
                same_login(&issue.owner, &now.owner)
                    && !issue.id.workspace.eq_ignore_ascii_case(&now.workspace)
            });
            other_viewer || other_workspace
        })
        .map(|v| v.owner.clone())
        .collect()
}

/// The Linear entries of a Linear survey's coverage: the sources it speaks
/// for.
fn linear_coverage(answered: &Coverage) -> Coverage {
    let mut coverage = Coverage::default();
    for entry in answered.entries().iter().filter(|e| e.source.is_linear()) {
        coverage.record(entry.source.clone(), entry.state.clone());
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
    /// Sources the refresh read completely (folded).
    complete: HashSet<Source>,
    /// Sources the refresh recorded, in any state (folded).
    recorded: HashSet<Source>,
    viewer_read: bool,
}

impl Reading {
    /// What a refresh of `owners` that recorded `coverage` read.
    fn of(owners: Vec<String>, coverage: &Coverage) -> Self {
        let entries = coverage.entries();
        Self {
            owners,
            complete: entries
                .iter()
                .filter(|e| e.state.is_complete())
                .map(|e| fold_source(&e.source))
                .collect(),
            recorded: entries.iter().map(|e| fold_source(&e.source)).collect(),
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
    /// A Repo the refresh recorded a source for is judged by that source alone:
    /// read completely, its facts are all there even if a later page of its
    /// Owner's list was lost. A Repo with no source of its own was read (or not)
    /// as part of its Owner's list, so only a complete Owner says it is all there.
    fn keeps_repo_fact(&self, repo: &RepoId, kind: SourceKind) -> bool {
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

    /// Should an earlier fact read for `owner` by its `kind` source (a Linear
    /// fact), absent from the refresh, be kept? Only when the refresh recorded
    /// that source and could not read it completely: a source it read
    /// completely has all its facts in the refresh, and one it didn't record
    /// is no longer read at all.
    fn keeps_owner_fact(&self, owner: &str, kind: SourceKind) -> bool {
        if self.owner_as_watched(owner).is_none() {
            return false;
        }
        let source = Source::Owner {
            owner: owner.to_ascii_lowercase(),
            kind,
        };
        self.recorded.contains(&source) && !self.complete.contains(&source)
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
        WorkItemId::LinearIssue { owner, issue } => WorkItemId::LinearIssue {
            owner: owner.to_ascii_lowercase(),
            issue,
        },
    }
}
