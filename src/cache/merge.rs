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

use super::{CachedObservation, Carried, LinearReads, SourceRead, source_owner};
use crate::{
    domain::{
        Coverage, CoverageEntry, LinearPart, LinearSourceKind, Observation, Repo, RepoId, Source,
        SourceKind, SourceState,
        facts::{Branch, Issue, MemberStateAttempt, PullRequest, PullRequestRef, Run},
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
    github_only(&mut fresh);
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

/// Merges a Repo Survey of `repo` into `previous` (spec 0003, ADR-0005). It
/// speaks for that Repo's sources only, by [`merge`]'s rule and read-time
/// guard. Everything else den holds (the Owner's other Repos and its own
/// sources, other Owners, the Linear part, the members' states) stays as it
/// was, each with its own age, and the watched Owners stay `previous`'s. The
/// GitHub facts of another GitHub viewer are never merged.
pub fn merge_repo(
    previous: Option<CachedObservation>,
    mut fresh: Observation,
    repo: &RepoId,
) -> CachedObservation {
    github_only(&mut fresh);
    let Some(mut previous) = previous else {
        return CachedObservation::fresh(fresh);
    };
    let linear = LinearKept::take(&mut previous);
    let mut merged = if previous.observation.viewer.is(&fresh.viewer.login) {
        let (inside, elsewhere) = Elsewhere::split(previous, repo);
        // The Repo's own Owner, so its earlier facts are judged by the rule
        // rather than dropped as an unwatched Owner's.
        fresh.owners = vec![repo.owner.clone()];
        let mut merged = merge_github(inside, fresh);
        elsewhere.put(&mut merged);
        merged
    } else {
        CachedObservation::fresh(fresh)
    };
    linear.put(&mut merged);
    merged
}

/// A GitHub survey says nothing about Linear or the members' states: whatever
/// of them it carries is dropped before merging.
fn github_only(fresh: &mut Observation) {
    fresh.linear = LinearPart::default();
    fresh.member_states.clear();
    fresh.member_attempts.clear();
    fresh
        .coverage
        .retain(|e| !e.source.is_linear() && !e.source.is_member_states());
}

/// What den holds that a Repo Survey doesn't speak for: every fact, coverage
/// entry, age and read time of other Repos and of Owners, and the watched
/// Owners, as of `held_at`.
struct Elsewhere {
    held_at: Timestamp,
    owners: Vec<String>,
    repos: Vec<Repo>,
    pull_requests: Vec<PullRequest>,
    issues: Vec<Issue>,
    branches: Vec<Branch>,
    runs: Vec<Run>,
    coverage: Vec<CoverageEntry>,
    carried: Vec<Carried>,
    reads: Vec<SourceRead>,
}

impl Elsewhere {
    /// Splits `previous` (without Linear data) into what `repo`'s Survey
    /// speaks for (its facts and Repo sources, with the viewer and the
    /// members' states, which [`merge_github`] handles) and the rest.
    fn split(mut previous: CachedObservation, repo: &RepoId) -> (CachedObservation, Self) {
        owner_aged_as_repo(&mut previous, repo);
        let ours = |source: &Source| match source {
            Source::Repo { repo: r, .. } => r.same(repo),
            Source::Viewer => true,
            Source::Owner { .. } => source.is_member_states(),
        };
        let o = &mut previous.observation;
        let (repos, mine) = std::mem::take(&mut o.repos)
            .into_iter()
            .partition(|r| !r.id.same(repo));
        o.repos = mine;
        let (pull_requests, mine) = std::mem::take(&mut o.pull_requests)
            .into_iter()
            .partition(|i| !i.repo.same(repo));
        o.pull_requests = mine;
        let (issues, mine) = std::mem::take(&mut o.issues)
            .into_iter()
            .partition(|i| !i.repo.same(repo));
        o.issues = mine;
        let (branches, mine) = std::mem::take(&mut o.branches)
            .into_iter()
            .partition(|i| !i.repo.same(repo));
        o.branches = mine;
        let (runs, mine) = std::mem::take(&mut o.runs)
            .into_iter()
            .partition(|i| !i.repo.same(repo));
        o.runs = mine;
        let coverage: Vec<CoverageEntry> = o
            .coverage
            .entries()
            .iter()
            .filter(|e| !ours(&e.source))
            .cloned()
            .collect();
        o.coverage.retain(|e| ours(&e.source));
        let (carried, mine) = std::mem::take(&mut previous.carried)
            .into_iter()
            .partition(|c| !ours(&c.source));
        previous.carried = mine;
        let (reads, mine) = std::mem::take(&mut previous.reads)
            .into_iter()
            .partition(|r| !ours(&r.source));
        previous.reads = mine;
        let elsewhere = Self {
            held_at: previous.observation.observed_at,
            owners: previous.observation.owners.clone(),
            repos,
            pull_requests,
            issues,
            branches,
            runs,
            coverage,
            carried,
            reads,
        };
        (previous, elsewhere)
    }

    /// Puts it back beside the merged Repo. Whichever is older carries its
    /// age, so the Observation is as new as its newest data and nothing reads
    /// newer than it is.
    fn put(self, merged: &mut CachedObservation) {
        let merged_at = merged.observation.observed_at;
        if self.held_at > merged_at {
            carry_uncarried(merged, merged_at);
            merged.observation.observed_at = self.held_at;
        }
        let o = &mut merged.observation;
        o.owners = self.owners;
        o.repos.extend(self.repos);
        o.pull_requests.extend(self.pull_requests);
        o.issues.extend(self.issues);
        o.branches.extend(self.branches);
        o.runs.extend(self.runs);
        for entry in &self.coverage {
            o.coverage.record(entry.source.clone(), entry.state.clone());
        }
        merged.carried.extend(self.carried);
        merged.reads.extend(self.reads);
        if merged_at > self.held_at {
            let older: Vec<Source> = self
                .coverage
                .into_iter()
                .filter(|e| has_data(&e.state))
                .map(|e| e.source)
                .filter(|s| !merged.carried.iter().any(|c| c.source.same(s)))
                .collect();
            merged
                .carried
                .extend(older.into_iter().map(|source| Carried {
                    source,
                    observed_at: self.held_at,
                }));
        }
    }
}

/// Gives `repo`'s sources that den judges by its Owner's (no coverage entry
/// of their own: an Owner survey cut off before the Repo, say) a copy of that
/// Owner source's coverage and age, as [`CachedObservation::carried_since`]
/// reads them. The Owner's sources stay behind when a Repo Survey splits
/// `previous`, and fitting the cache drops them when the Owner is no longer
/// watched but the Repo is open, so without this the Repo's kept facts would
/// lose their age and read as new.
pub(super) fn owner_aged_as_repo(previous: &mut CachedObservation, repo: &RepoId) {
    let own = |kind: SourceKind| Source::Repo {
        repo: repo.clone(),
        kind,
    };
    let owner_entries: Vec<CoverageEntry> = previous
        .observation
        .coverage
        .entries()
        .iter()
        .filter(|e| {
            matches!(&e.source, Source::Owner { owner, .. } if same_login(owner, &repo.owner))
                && !e.source.is_linear()
                && !e.source.is_member_states()
        })
        .cloned()
        .collect();
    for entry in owner_entries {
        let Source::Owner { kind, .. } = entry.source else {
            continue;
        };
        let source = own(kind);
        let recorded = |cached: &CachedObservation| {
            cached
                .observation
                .coverage
                .entries()
                .iter()
                .any(|e| e.source.same(&source))
        };
        if recorded(previous) || previous.carried.iter().any(|c| c.source.same(&source)) {
            continue;
        }
        if let Some(age) = previous
            .carried
            .iter()
            .find(|c| c.source.same(&entry.source))
            .map(|c| c.observed_at)
        {
            previous.carried.push(Carried {
                source: source.clone(),
                observed_at: age,
            });
        }
        previous.observation.coverage.record(source, entry.state);
    }
}

/// Every source of `cached` holding data and no age of its own yet carries
/// `at`, the time its Observation is about to stop being.
fn carry_uncarried(cached: &mut CachedObservation, at: Timestamp) {
    let older: Vec<Source> = cached
        .observation
        .coverage
        .entries()
        .iter()
        .filter(|e| {
            has_data(&e.state) && e.source != Source::Viewer && !e.source.is_member_states()
        })
        .map(|e| e.source.clone())
        .filter(|s| !cached.carried.iter().any(|c| c.source.same(s)))
        .collect();
    cached
        .carried
        .extend(older.into_iter().map(|source| Carried {
            source,
            observed_at: at,
        }));
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
///
/// The read-time guard (spec 0003, "Merge"): for each Repo and source, a
/// read is merged only if it didn't start before the one den holds. One that
/// started earlier changes nothing for that Repo and source, complete or
/// partial: not its facts, its coverage, its age or its read time.
fn merge_github(previous: CachedObservation, mut fresh: Observation) -> CachedObservation {
    let guarded = Guarded::of(&previous, &fresh);
    guarded.withdraw(&mut fresh, &previous);
    let fresh_at = fresh.observed_at;
    let mut merged = fresh;
    // Borrows the refresh's coverage; the merge below only moves other fields.
    let reading = Reading::of(merged.owners.clone(), &merged.coverage).guarding(&guarded);
    let carried = carried_sources(&previous, &merged, &reading);
    let reads = merged_reads(&previous.reads, &merged.coverage, &guarded, fresh_at);
    let held_at = previous.observation.observed_at;
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
    let mut cached = CachedObservation {
        observation: merged,
        carried,
        reads,
    };
    if !guarded.is_empty() && held_at > fresh_at {
        as_new_as_held(&mut cached, held_at, fresh_at, &guarded);
    }
    cached
}

/// The Repo sources a refresh may not replace (the read-time guard): den
/// holds a read of them that started after the refresh did. Keyed by folded
/// Repo and the kind that lists the facts; details go with their list.
#[derive(Debug, Default)]
struct Guarded(Vec<(RepoId, SourceKind)>);

/// Guards nothing: for merges with no Repo reads to compare (Linear's).
static GUARDS_NOTHING: Guarded = Guarded(Vec::new());

impl Guarded {
    /// Every held Repo read the refresh speaks for: the sources it recorded,
    /// and the held reads of its Owners' Repos it never reached. An Owner
    /// survey cut off before a Repo (interrupted, or paused at the rate-limit
    /// floor) records nothing for it, yet still replaces the Owner's coverage,
    /// so without the second list that Repo's held coverage and read time
    /// would go while its facts stayed.
    fn of(previous: &CachedObservation, fresh: &Observation) -> Self {
        let recorded = fresh.coverage.entries().iter().map(|e| &e.source);
        let unreached = previous.reads.iter().map(|r| &r.source).filter(|s| {
            matches!(s, Source::Repo { repo, .. }
                if fresh.owners.iter().any(|o| same_login(o, &repo.owner)))
        });
        let mut held: Vec<(RepoId, SourceKind)> = Vec::new();
        for source in recorded.chain(unreached) {
            let Source::Repo { repo, kind } = source else {
                continue;
            };
            let key = (repo.folded(), listing(*kind));
            let newer = previous
                .read_started(repo, key.1)
                .is_some_and(|started| started > fresh.observed_at);
            if newer && !held.contains(&key) {
                held.push(key);
            }
        }
        Self(held)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `repo`'s `kind` facts (or their details) stay as den held them.
    fn holds(&self, repo: &RepoId, kind: SourceKind) -> bool {
        self.0.contains(&(repo.folded(), listing(kind)))
    }

    fn holds_source(&self, source: &Source) -> bool {
        matches!(source, Source::Repo { repo, kind } if self.holds(repo, *kind))
    }

    /// Takes what den holds back out of `fresh`: its facts of a guarded Repo
    /// and source go, and its coverage of them becomes the held coverage.
    fn withdraw(&self, fresh: &mut Observation, previous: &CachedObservation) {
        if self.is_empty() {
            return;
        }
        fresh
            .repos
            .retain(|r| !self.holds(&r.id, SourceKind::Repos));
        fresh
            .pull_requests
            .retain(|i| !self.holds(&i.repo, SourceKind::PullRequests));
        fresh
            .issues
            .retain(|i| !self.holds(&i.repo, SourceKind::Issues));
        fresh
            .branches
            .retain(|i| !self.holds(&i.repo, SourceKind::Branches));
        fresh
            .runs
            .retain(|i| !self.holds(&i.repo, SourceKind::Runs));
        fresh.coverage.retain(|e| !self.holds_source(&e.source));
        for entry in previous.observation.coverage.entries() {
            if self.holds_source(&entry.source) {
                fresh
                    .coverage
                    .record(entry.source.clone(), entry.state.clone());
            }
        }
    }
}

/// Whether a source in this state holds facts den shows: one that failed or
/// wasn't fetched holds none of its own (kept ones carry their age already).
fn has_data(state: &SourceState) -> bool {
    matches!(state, SourceState::Complete | SourceState::Partial(_))
}

/// The source that lists a kind's facts: a PR's or an Issue's details are
/// read with its list, so they are guarded with it.
fn listing(kind: SourceKind) -> SourceKind {
    match kind {
        SourceKind::PullRequestDetails => SourceKind::PullRequests,
        SourceKind::IssueDetails => SourceKind::Issues,
        other => other,
    }
}

/// When the read behind each merged Repo source started: the held read for a
/// guarded source, the refresh's start for every other one it recorded.
fn merged_reads(
    held: &[SourceRead],
    merged: &Coverage,
    guarded: &Guarded,
    fresh_at: Timestamp,
) -> Vec<SourceRead> {
    let mut reads: Vec<SourceRead> = held
        .iter()
        .filter(|r| guarded.holds_source(&r.source))
        .cloned()
        .collect();
    reads.extend(
        merged
            .entries()
            .iter()
            .filter(|e| matches!(e.source, Source::Repo { .. }) && !guarded.holds_source(&e.source))
            .map(|e| SourceRead {
                source: e.source.clone(),
                started_at: fresh_at,
            }),
    );
    reads
}

/// A guarded read keeps the Observation as new as the one den held, so a
/// merge never reads older than what it shows (and the cache file, which
/// refuses an older survey, still takes it). The refresh's own sources then
/// carry the refresh's age.
fn as_new_as_held(
    cached: &mut CachedObservation,
    held_at: Timestamp,
    fresh_at: Timestamp,
    guarded: &Guarded,
) {
    cached.observation.observed_at = held_at;
    let older: Vec<Source> = cached
        .observation
        .coverage
        .entries()
        .iter()
        .filter(|e| has_data(&e.state))
        .map(|e| &e.source)
        .filter(|s| **s != Source::Viewer && !s.is_member_states() && !guarded.holds_source(s))
        .filter(|s| !cached.carried.iter().any(|c| c.source.same(s)))
        .cloned()
        .collect();
    cached
        .carried
        .extend(older.into_iter().map(|source| Carried {
            source,
            observed_at: fresh_at,
        }));
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
        reads,
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
            reads,
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
    /// The Repo sources den holds a newer read of: their earlier facts stay.
    guarded: &'f Guarded,
}

impl<'f> Reading<'f> {
    /// What a refresh of `owners` that recorded `coverage` read.
    fn of(owners: Vec<String>, coverage: &'f Coverage) -> Self {
        Self {
            owners,
            coverage,
            viewer_read: coverage.get(&Source::Viewer) == Some(&SourceState::Complete),
            guarded: &GUARDS_NOTHING,
        }
    }

    /// The same, keeping the earlier facts of `guarded` Repo sources.
    fn guarding(mut self, guarded: &'f Guarded) -> Self {
        self.guarded = guarded;
        self
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
        if self.guarded.holds(repo, kind) {
            return true;
        }
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
