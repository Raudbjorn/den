//! The cache: the last Observation per viewer, kept on disk so den starts from
//! it instantly, and merged with each refresh by source (spec, "Cache").
//!
//! - [`CachedObservation`]: the Observation den shows, plus the age of any data it
//!   carried over from an earlier survey because the latest one could not read
//!   that source.
//! - [`merge`] (a GitHub survey), [`merge_linear`] (a Linear survey),
//!   [`merge_member_states`] (Landing Group members' states, #37) and
//!   [`settle_member_states`] (those states against the groups shown): pure. A
//!   source the refresh read completely is replaced; a source it could not read
//!   (failed, partial, not fetched) keeps the earlier data, marked with that
//!   data's own age. One error never wipes the list, and a refresh of one service
//!   never touches the other's data. Linear data is dropped when the Linear
//!   viewer behind it changes, or when its Linear source is no longer read.
//! - [`FileCache`]: the real [`ObservationCache`](crate::ports::ObservationCache),
//!   one versioned JSON file per viewer under `$XDG_STATE_HOME/den/cache/`.
//!
//! The merge is generic over sources: it works from coverage entries and each
//! fact's Repo, so adding a Work Item kind never needs to touch it.

mod file;
mod merge;

pub use file::{CACHE_FORMAT, CACHE_SUBDIR, FileCache, MAX_CACHE_BYTES};
pub use merge::{
    LinearMerge, merge, merge_linear, merge_member_states, merge_repo, settle_member_states,
};

use crate::domain::{
    LinearSourceKind, Observation, RepoId, Source, SourceKind, SourceState, ids::same_login,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Data den kept for a source the latest survey could not read completely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Carried {
    pub source: Source,
    /// When the kept data was observed: older than the survey of its service
    /// (the Observation's own time for GitHub, its Linear part's for Linear).
    pub observed_at: Timestamp,
}

/// When the read behind one Repo source's coverage started (cache format 13,
/// ADR-0005): what the read-time guard compares a later answer with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRead {
    pub source: Source,
    pub started_at: Timestamp,
}

/// The Observation den shows: the latest survey, with earlier data kept for the
/// sources it could not read. Its coverage is always the latest survey's, so a
/// kept source still reads as failed or partial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedObservation {
    pub observation: Observation,
    /// One entry per source whose shown data is older than its service's latest
    /// survey (`observation.observed_at`, or `observation.linear.observed_at`
    /// for a Linear source).
    pub carried: Vec<Carried>,
    /// One entry per Repo source in the coverage: when the read whose
    /// coverage den holds for it started (spec 0003, "Cache").
    #[serde(default)]
    pub reads: Vec<SourceRead>,
}

impl CachedObservation {
    /// A survey with nothing carried over: each Repo source it recorded was
    /// read when it started.
    pub fn fresh(observation: Observation) -> Self {
        let reads = observation
            .coverage
            .entries()
            .iter()
            .filter(|e| matches!(e.source, Source::Repo { .. }))
            .map(|e| SourceRead {
                source: e.source.clone(),
                started_at: observation.observed_at,
            })
            .collect();
        Self {
            observation,
            carried: Vec::new(),
            reads,
        }
    }

    /// When the read whose coverage den holds for `repo`'s `kind` started, if
    /// den holds one. Repo names compare as GitHub compares them.
    pub fn read_started(&self, repo: &RepoId, kind: SourceKind) -> Option<Timestamp> {
        self.reads
            .iter()
            .filter(|r| matches!(&r.source, Source::Repo { repo: held, kind: k } if *k == kind && held.same(repo)))
            .map(|r| r.started_at)
            .max()
    }

    /// When the data shown for `source` was observed, if it is older than the
    /// Observation itself. A Repo's source without a coverage entry of its
    /// own falls back to its Owner's (spec 0003, "Coverage per Repo").
    pub fn carried_since(&self, source: &Source) -> Option<Timestamp> {
        let own = self
            .carried
            .iter()
            .find(|c| c.source.same(source))
            .map(|c| c.observed_at);
        let recorded = |source: &Source| {
            self.observation
                .coverage
                .entries()
                .iter()
                .any(|e| e.source.same(source))
        };
        own.or_else(|| match source {
            Source::Repo { .. } if recorded(source) => None,
            Source::Repo { repo, kind } => self.carried_since(&Source::Owner {
                owner: repo.owner.clone(),
                kind: *kind,
            }),
            Source::Owner { .. } | Source::Viewer => None,
        })
    }

    /// Fits a cached Observation to the configured Owners: drops what belongs to
    /// Owners no longer watched, and marks newly watched Owners as not fetched yet
    /// so their absence never reads as a complete, empty survey. The `open`
    /// Repo's facts and Repo sources stay even when its Owner isn't watched:
    /// den opens Repos outside the watched Owners too (spec 0003, "Startup").
    /// Facts aged only through that Owner's sources take the Owner's coverage
    /// and age as the Repo's own first, so they don't read as new once the
    /// Owner's sources go.
    pub fn restricted_to(mut self, owners: &[String], open: Option<&RepoId>) -> Self {
        let watched = |owner: &str| owners.iter().any(|o| o.eq_ignore_ascii_case(owner));
        if let Some(repo) = open
            && !watched(&repo.owner)
            && self.observation.repos.iter().any(|r| r.id.same(repo))
        {
            merge::owner_aged_as_repo(&mut self, repo);
        }
        let kept = |repo: &RepoId| watched(&repo.owner) || open.is_some_and(|o| o.same(repo));
        let source_kept = |source: &Source| match source {
            Source::Repo { repo, .. } => kept(repo),
            other => source_owner(other).is_none_or(watched),
        };
        let observation = &mut self.observation;
        observation.repos.retain(|r| kept(&r.id));
        observation.pull_requests.retain(|i| kept(&i.repo));
        observation.issues.retain(|i| kept(&i.repo));
        observation.branches.retain(|i| kept(&i.repo));
        observation.runs.retain(|i| kept(&i.repo));
        observation.linear.issues.retain(|i| watched(&i.owner));
        observation.linear.viewers.retain(|v| watched(&v.owner));
        observation.linear.team_states.retain(|t| watched(&t.owner));
        observation
            .member_states
            .retain(|m| watched(&m.pr.repo.owner));
        observation
            .member_attempts
            .retain(|a| watched(&a.pr.repo.owner));
        let mut coverage = crate::domain::Coverage::default();
        for entry in observation.coverage.entries() {
            if source_kept(&entry.source) {
                coverage.record(entry.source.clone(), entry.state.clone());
            }
        }
        for owner in owners {
            let surveyed = observation
                .owners
                .iter()
                .any(|o| o.eq_ignore_ascii_case(owner));
            if !surveyed {
                coverage.record(
                    Source::Owner {
                        owner: owner.clone(),
                        kind: SourceKind::Repos,
                    },
                    SourceState::NotYetFetched,
                );
            }
        }
        observation.coverage = coverage;
        observation.owners = owners.to_vec();
        self.carried.retain(|c| source_kept(&c.source));
        self.reads.retain(|r| source_kept(&r.source));
        self
    }
}

/// What den reads from Linear for one Owner's Linear Workflow: the workspace,
/// and the Linear sources its config asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearReads<'a> {
    pub owner: &'a str,
    pub workspace: &'a str,
    pub sources: Vec<LinearSourceKind>,
}

impl CachedObservation {
    /// Fits a cached Linear part to the configured Linear Workflows: drops the
    /// Linear facts, Linear coverage and Linear ages of an Owner that has no
    /// Linear Workflow now, or whose workflow names another workspace than the
    /// cached data was read in, and of every Linear source the config no
    /// longer reads (a fact goes with the source that listed it).
    pub fn restricted_to_linear(mut self, workflows: &[LinearReads<'_>]) -> Self {
        let workflow_of = |owner: &str| workflows.iter().find(|w| same_login(w.owner, owner));
        let mut dropped: Vec<String> = Vec::new();
        for (owner, workspace) in self.observation.linear.workspaces() {
            let fits =
                workflow_of(owner).is_some_and(|w| w.workspace.eq_ignore_ascii_case(workspace));
            if !fits && !dropped.iter().any(|d| same_login(d, owner)) {
                dropped.push(owner.to_owned());
            }
        }
        let reads = |owner: &str, kind: LinearSourceKind| {
            !dropped.iter().any(|d| same_login(d, owner))
                && workflow_of(owner).is_some_and(|w| w.sources.contains(&kind))
        };
        let part = &mut self.observation.linear;
        part.viewers
            .retain(|v| reads(&v.owner, LinearSourceKind::Viewer));
        part.issues.retain(|i| reads(&i.owner, i.listed_by));
        part.team_states
            .retain(|t| reads(&t.owner, LinearSourceKind::TeamStates));
        let linear_kept = |source: &Source| match source {
            Source::Owner {
                owner,
                kind: SourceKind::Linear(kind),
            } => reads(owner, *kind),
            Source::Owner { .. } | Source::Repo { .. } | Source::Viewer => true,
        };
        self.observation.coverage.retain(|e| linear_kept(&e.source));
        self.carried.retain(|c| linear_kept(&c.source));
        self
    }
}

/// The Owner a source belongs to; `None` for the viewer.
pub(crate) fn source_owner(source: &Source) -> Option<&str> {
    match source {
        Source::Viewer => None,
        Source::Owner { owner, .. } => Some(owner),
        Source::Repo { repo, .. } => Some(&repo.owner),
    }
}

/// True only when the survey read every source completely: the viewer, and at
/// least one source of every watched Owner, with nothing failed, partial or not
/// fetched. Only then may den say "Nothing needs you".
pub fn survey_complete(observation: &Observation) -> bool {
    let coverage = &observation.coverage;
    let viewer_read = coverage.get(&Source::Viewer).is_some();
    let every_owner_read = observation.owners.iter().all(|owner| {
        coverage
            .entries()
            .iter()
            .any(|e| source_owner(&e.source).is_some_and(|o| o.eq_ignore_ascii_case(owner)))
    });
    viewer_read && every_owner_read && coverage.is_complete()
}

/// Why the cache could not be read or written. Never fatal: den logs it and
/// surveys afresh.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    #[error("cannot read the cache {}: {detail}", path.display())]
    Read { path: PathBuf, detail: String },
    #[error("the cache {} is not one den can read ({detail})", path.display())]
    Unreadable { path: PathBuf, detail: String },
    #[error(
        "the cache {} has format {found}, this den reads format {expected}",
        path.display()
    )]
    Format {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[error(
        "the cache {} is {bytes} bytes, more than den's limit of {limit} bytes",
        path.display()
    )]
    TooLarge {
        path: PathBuf,
        bytes: u64,
        limit: u64,
    },
    #[error("cannot write the cache {}: {detail}", path.display())]
    Write { path: PathBuf, detail: String },
    #[error(
        "did not replace the cache {}: it holds a newer survey ({stored}) than this one ({offered})",
        path.display()
    )]
    Superseded {
        path: PathBuf,
        stored: Timestamp,
        offered: Timestamp,
    },
    #[error("{login:?} is not a GitHub login den can name a cache file after")]
    BadViewer { login: String },
}
