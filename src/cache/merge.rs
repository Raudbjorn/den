//! Merging a refresh into what den already shows, by source.
//!
//! A fact from the refresh always wins. An earlier fact the refresh did not
//! return is dropped only when the refresh read its source completely (then it
//! is really gone); otherwise it is kept, and the source is marked as carried
//! with the age of the kept data. Coverage is always the refresh's, so a kept
//! source still reads as failed or partial: den never presents old data as new.
//!
//! GitHub and Linear are surveyed apart, so each refresh speaks for its own
//! sources only. The Landing Group members' states (#37), read by number
//! after both surveys, are a third: a GitHub survey of the same GitHub viewer
//! keeps them and their coverage as they were, and [`merge_member_states`]
//! replaces each member's state it read. [`merge`] takes a GitHub survey and
//! leaves the Linear part, its coverage and its ages as they were, also when
//! it starts afresh for another GitHub viewer (Linear data is read as the
//! Linear viewer, not the GitHub one); [`merge_linear`] takes a Linear survey
//! and leaves GitHub's alone. Both apply the one rule above ([`Reading`],
//! [`keep_unread`]): a fact names its source, a GitHub fact by its kind and
//! Repo, a Linear fact by its Owner and the Linear source that listed it. A
//! Linear fact list added later (a start team's states, say) is one more
//! [`keep_unread`] call in [`merge_linear`], and one more `retain` by its
//! source in [`CachedObservation::restricted_to_linear`].
//!
//! A Linear survey speaks for the Linear sources it records and for every one
//! the config reads ([`LinearReads`]): one of those it has no word on is not
//! yet fetched, so its earlier data is kept with its age. A Linear source that
//! is neither is no longer read: its coverage, its age and the facts it listed
//! are dropped, so it never stays "not yet fetched" nor keeps its facts for
//! good. The rule is
//! the one above, by source, so it holds for every Linear source kind.
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

use super::{CachedObservation, Carried, LinearReads, source_owner};
use crate::{
    domain::{
        Coverage, CoverageEntry, LinearPart, LinearSourceKind, Observation, RepoId, Source,
        SourceKind, SourceState,
        facts::{MemberStateAttempt, PullRequestRef},
        ids::same_login,
    },
    ports::{LinearSurvey, MemberStatesRead},
};
use jiff::Timestamp;
use std::{collections::HashSet, hash::Hash};

/// A Linear survey merged into what den showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearMerge {
    pub cached: CachedObservation,
    /// The Owners whose Linear viewer the survey read as someone else than
    /// the earlier Linear data was read as: all of their earlier Linear data
    /// was dropped.
    pub viewer_changed: Vec<String>,
}

/// Merges a GitHub survey `fresh` into `previous`, GitHub source by GitHub
/// source. The GitHub facts of a previous Observation of another GitHub
/// viewer are never merged: the cache is per viewer. The Linear part, its
/// coverage and its ages stay as `previous` had them, whoever the GitHub
/// viewer is: Linear data is read as the Linear viewer, and [`merge_linear`]
/// drops it when that one changes. Whatever Linear part or Linear coverage
/// `fresh` carries is ignored: a GitHub survey says nothing about Linear.
pub fn merge(previous: Option<CachedObservation>, mut fresh: Observation) -> CachedObservation {
    fresh.linear = LinearPart::default();
    fresh.member_states.clear();
    fresh.member_attempts.clear();
    fresh
        .coverage
        .retain(|e| !e.source.is_linear() && !e.source.is_member_states());
    let Some(mut previous) = previous else {
        return CachedObservation::fresh(fresh);
    };
    let linear = LinearKept::take(&mut previous);
    let mut merged = if previous.observation.viewer.is(&fresh.viewer.login) {
        merge_github(previous, fresh)
    } else {
        CachedObservation::fresh(fresh)
    };
    linear.put(&mut merged);
    merged
}

/// The Linear part of what den showed, its Linear coverage and the ages of
/// its kept Linear data: what a GitHub survey leaves as it was.
struct LinearKept {
    part: LinearPart,
    coverage: Vec<CoverageEntry>,
    carried: Vec<Carried>,
}

impl LinearKept {
    /// Takes the Linear part, coverage and ages out of `from`, leaving GitHub's.
    fn take(from: &mut CachedObservation) -> Self {
        let part = std::mem::take(&mut from.observation.linear);
        let coverage = from
            .observation
            .coverage
            .entries()
            .iter()
            .filter(|e| e.source.is_linear())
            .cloned()
            .collect();
        from.observation.coverage.retain(|e| !e.source.is_linear());
        let (carried, github) = std::mem::take(&mut from.carried)
            .into_iter()
            .partition(|c| c.source.is_linear());
        from.carried = github;
        Self {
            part,
            coverage,
            carried,
        }
    }

    /// Puts them into `into`, which holds no Linear data.
    fn put(self, into: &mut CachedObservation) {
        into.observation.linear = self.part;
        for entry in self.coverage {
            into.observation.coverage.record(entry.source, entry.state);
        }
        into.carried.extend(self.carried);
    }
}

/// Merges `fresh` into `previous` of the same GitHub viewer; neither holds
/// Linear data.
fn merge_github(previous: CachedObservation, fresh: Observation) -> CachedObservation {
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
    if !reading.viewer_read {
        for team in previous_observation.viewer.teams {
            if !merged.viewer.belongs_to(&team) {
                merged.viewer.teams.push(team);
            }
        }
    }
    // The members' states stay as they were, with their coverage: they are
    // read by number after the survey, not by it (#37). They are GitHub data,
    // so unlike the Linear part they go when the GitHub viewer changes.
    merged.member_states = previous_observation.member_states;
    merged.member_attempts = previous_observation.member_attempts;
    for entry in previous_observation.coverage.entries() {
        if entry.source.is_member_states() {
            merged
                .coverage
                .record(entry.source.clone(), entry.state.clone());
        }
    }
    CachedObservation {
        observation: merged,
        carried,
    }
}

/// Merges a read of Landing Group members' states (#37) into `previous`: each
/// member it read a state for replaces that member's earlier state, any other
/// keeps its earlier one, and the read's coverage replaces the earlier
/// coverage of the same Owners' member states. A member den asked about and
/// got no state for keeps the one it had, if any (the coverage says the read
/// was incomplete): a failed read never invents a state, and never erases a
/// merge, which is final on GitHub.
///
/// A member it sent to GitHub and got no state for is recorded as attempted
/// `now` (`Observation.member_attempts`), so the next bounded read asks for
/// the others first; one it got a state for is no longer an attempt.
pub fn merge_member_states(
    mut previous: CachedObservation,
    read: MemberStatesRead,
    now: Timestamp,
) -> CachedObservation {
    let observation = &mut previous.observation;
    let answered = |pr: &PullRequestRef| read.states.iter().any(|s| s.pr.is(&pr.repo, pr.number));
    let unanswered: Vec<&PullRequestRef> = read.asked.iter().filter(|pr| !answered(pr)).collect();
    observation.member_attempts.retain(|a| {
        !answered(&a.pr) && !unanswered.iter().any(|pr| pr.is(&a.pr.repo, a.pr.number))
    });
    observation
        .member_attempts
        .extend(unanswered.into_iter().map(|pr| MemberStateAttempt {
            pr: pr.clone(),
            attempted_at: now,
        }));
    observation.member_states.retain(|kept| {
        !read
            .states
            .iter()
            .any(|s| s.pr.is(&kept.pr.repo, kept.pr.number))
    });
    observation.member_states.extend(read.states);
    let answered: Vec<Source> = read
        .coverage
        .entries()
        .iter()
        .map(|e| e.source.clone())
        .collect();
    observation
        .coverage
        .retain(|e| !answered.iter().any(|a| a.same(&e.source)));
    for entry in read.coverage.entries() {
        observation
            .coverage
            .record(entry.source.clone(), entry.state.clone());
    }
    previous
}

/// Settles the members' states (#37) once both surveys of a round have
/// answered, against the Landing Groups now decided: `members` are every
/// member of them, `wanted` the ones den is about to read the state of.
///
/// - A state of a Pull Request no longer a member of any group is dropped,
///   so the states kept (in memory and in the cache file) never outgrow the
///   groups den shows.
/// - An attempt (a member asked about and not answered) is dropped once den
///   no longer wants that member's state, so attempts never outgrow the
///   members still to read.
/// - An Owner's member-state coverage is dropped when den wants none of its
///   members' states: no read will replace it, and an old failure must not
///   leave the picture incomplete for good. An Owner den is about to read
///   keeps its entry until that read answers.
pub fn settle_member_states(
    mut previous: CachedObservation,
    members: &[PullRequestRef],
    wanted: &[PullRequestRef],
) -> CachedObservation {
    let fold = |pr: &PullRequestRef| {
        (
            pr.repo.owner.to_ascii_lowercase(),
            pr.repo.name.to_ascii_lowercase(),
            pr.number,
        )
    };
    let current: HashSet<_> = members.iter().map(fold).collect();
    let still_wanted: HashSet<_> = wanted.iter().map(fold).collect();
    let observation = &mut previous.observation;
    observation
        .member_states
        .retain(|kept| current.contains(&fold(&kept.pr)));
    observation
        .member_attempts
        .retain(|a| still_wanted.contains(&fold(&a.pr)));
    observation.coverage.retain(|e| match &e.source {
        Source::Owner {
            owner,
            kind: SourceKind::MemberStates,
        } => wanted.iter().any(|pr| same_login(&pr.repo.owner, owner)),
        _ => true,
    });
    previous
}

/// Merges a Linear survey into `previous`, Linear source by Linear source;
/// GitHub's part, coverage and ages stay as they were.
///
/// The survey speaks for the Linear sources it records and for every one the
/// config reads (`reads`): one of those it has no word on is not yet fetched
/// (its earlier data is kept with its age). Earlier Linear data of a source
/// that is neither, and so of an Owner without a Linear Workflow, is no
/// longer read and is dropped with that source's coverage and age.
///
/// A survey that isn't dated (Linear wasn't asked, before den knew the time)
/// leaves the Linear part's date as it was.
pub fn merge_linear(
    previous: CachedObservation,
    survey: LinearSurvey,
    reads: &[LinearReads<'_>],
) -> LinearMerge {
    let LinearSurvey {
        part: fresh,
        coverage: answered,
    } = survey;
    let coverage = read_coverage(answered, reads);
    let workflow_owners = reads.iter().map(|r| r.owner.to_owned()).collect();
    let viewer_changed = changed_linear_viewers(&previous.observation.linear, &fresh);
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
    earlier.team_states.retain(|t| !reread(&t.owner));
    let mut merged = fresh;
    merged.observed_at = merged.observed_at.or(earlier.observed_at);
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
/// an earlier Linear Issue or team's states name the other workspace). Their
/// earlier Linear data was read for another account, so none of it may be
/// kept.
fn changed_linear_viewers(previous: &LinearPart, fresh: &LinearPart) -> Vec<String> {
    fresh
        .viewers
        .iter()
        .filter(|now| {
            let other_user = previous
                .viewers
                .iter()
                .any(|before| same_login(&before.owner, &now.owner) && before.id != now.id);
            let other_workspace = previous.workspaces().any(|(owner, workspace)| {
                same_login(owner, &now.owner) && !workspace.eq_ignore_ascii_case(&now.workspace)
            });
            other_user || other_workspace
        })
        .map(|v| v.owner.clone())
        .collect()
}

/// The Linear sources a survey speaks for: every Linear source it recorded
/// for an Owner with a Linear Workflow, and every one `reads` asks for that it
/// has no word on, as not yet fetched. So a source the config reads is never
/// dropped for the survey's silence, whoever built the survey.
fn read_coverage(answered: Coverage, reads: &[LinearReads<'_>]) -> Coverage {
    let has_workflow = |owner: &str| reads.iter().any(|r| same_login(r.owner, owner));
    let mut coverage = answered;
    coverage.retain(|e| e.source.is_linear() && source_owner(&e.source).is_some_and(has_workflow));
    for read in reads {
        for kind in &read.sources {
            let source = Source::Owner {
                owner: read.owner.to_owned(),
                kind: SourceKind::Linear(*kind),
            };
            if !coverage.entries().iter().any(|e| e.source.same(&source)) {
                coverage.record(source, SourceState::NotYetFetched);
            }
        }
    }
    coverage
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
    /// fact), absent from the refresh, be kept? Only when the refresh recorded
    /// that source and could not read it completely: a source it read
    /// completely has all its facts in the refresh, and one it didn't record
    /// is no longer read at all.
    fn keeps_owner_fact(&self, owner: &str, kind: SourceKind) -> bool {
        if self.owner_as_watched(owner).is_none() {
            return false;
        }
        self.owner_source(owner, kind)
            .is_some_and(|state| !state.is_complete())
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
    reading: &Reading<'_>,
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
    reading: &Reading<'_>,
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
