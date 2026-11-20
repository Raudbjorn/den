//! The cache: the last Observation per viewer, kept on disk so den starts from
//! it instantly, and merged with each refresh by source (spec, "Cache").
//!
//! - [`CachedObservation`]: the Observation den shows, plus the age of any data it
//!   carried over from an earlier survey because the latest one could not read
//!   that source.
//! - [`merge`] (a GitHub survey), [`merge_linear`] (a Linear survey) and
//!   [`merge_member_states`] (Landing Group members' states, #37): pure. A
//!   source the refresh read completely is replaced; a source it could not read
//!   (failed, partial, not fetched) keeps the earlier data, marked with that
//!   data's own age. One error never wipes the list, and a refresh of one service
//!   never touches the other's data. Linear data is dropped when the Linear
//!   viewer behind it changes.
//! - [`FileCache`]: the real [`ObservationCache`](crate::ports::ObservationCache),
//!   one versioned JSON file per viewer under `$XDG_STATE_HOME/den/cache/`.
//!
//! The merge is generic over sources: it works from coverage entries and each
//! fact's Repo, so adding a Work Item kind never needs to touch it.

mod file;
mod merge;

pub use file::{CACHE_FORMAT, CACHE_SUBDIR, FileCache, MAX_CACHE_BYTES};
pub use merge::{changed_linear_viewers, merge, merge_linear, merge_member_states};

use crate::domain::{Observation, Source, SourceKind, SourceState};
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
}

impl CachedObservation {
    /// A survey with nothing carried over.
    pub fn fresh(observation: Observation) -> Self {
        Self {
            observation,
            carried: Vec::new(),
        }
    }

    /// When the data shown for `source` was observed, if it is older than the
    /// Observation itself. A Repo's source falls back to its Owner's.
    pub fn carried_since(&self, source: &Source) -> Option<Timestamp> {
        let own = self
            .carried
            .iter()
            .find(|c| c.source.same(source))
            .map(|c| c.observed_at);
        own.or_else(|| match source {
            Source::Repo { repo, kind } => self.carried_since(&Source::Owner {
                owner: repo.owner.clone(),
                kind: *kind,
            }),
            Source::Owner { .. } | Source::Viewer => None,
        })
    }

    /// Fits a cached Observation to the configured Owners: drops what belongs to
    /// Owners no longer watched, and marks newly watched Owners as not fetched yet
    /// so their absence never reads as a complete, empty survey.
    pub fn restricted_to(mut self, owners: &[String]) -> Self {
        let watched = |owner: &str| owners.iter().any(|o| o.eq_ignore_ascii_case(owner));
        let observation = &mut self.observation;
        observation.repos.retain(|r| watched(&r.id.owner));
        observation.pull_requests.retain(|i| watched(&i.repo.owner));
        observation.issues.retain(|i| watched(&i.repo.owner));
        observation.branches.retain(|i| watched(&i.repo.owner));
        observation.runs.retain(|i| watched(&i.repo.owner));
        observation.linear.issues.retain(|i| watched(&i.owner));
        observation.linear.viewers.retain(|v| watched(&v.owner));
        observation.linear.team_states.retain(|t| watched(&t.owner));
        observation
            .member_states
            .retain(|m| watched(&m.pr.repo.owner));
        let mut coverage = crate::domain::Coverage::default();
        for entry in observation.coverage.entries() {
            if source_owner(&entry.source).is_none_or(watched) {
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
        self.carried
            .retain(|c| source_owner(&c.source).is_none_or(watched));
        self
    }
}

impl CachedObservation {
    /// Fits a cached Linear part to the configured Linear Workflows, given each
    /// one's `(Owner, workspace)`: drops the Linear facts, Linear coverage and
    /// Linear ages of an Owner that has no Linear Workflow now, or whose
    /// workflow names another workspace than the cached data was read in.
    pub fn restricted_to_linear(mut self, workflows: &[(&str, &str)]) -> Self {
        let workspace_of = |owner: &str| {
            workflows
                .iter()
                .find(|(o, _)| o.eq_ignore_ascii_case(owner))
                .map(|(_, w)| *w)
        };
        let part = &self.observation.linear;
        let mut dropped: Vec<String> = Vec::new();
        let read_elsewhere = part
            .viewers
            .iter()
            .map(|v| (&v.owner, &v.workspace))
            .chain(part.issues.iter().map(|i| (&i.owner, &i.id.workspace)))
            .chain(part.team_states.iter().map(|t| (&t.owner, &t.workspace)));
        for (owner, workspace) in read_elsewhere {
            let fits = workspace_of(owner).is_some_and(|w| w.eq_ignore_ascii_case(workspace));
            if !fits && !dropped.iter().any(|d| d.eq_ignore_ascii_case(owner)) {
                dropped.push(owner.clone());
            }
        }
        let keep = |owner: &str| {
            workspace_of(owner).is_some() && !dropped.iter().any(|d| d.eq_ignore_ascii_case(owner))
        };
        let part = &mut self.observation.linear;
        part.viewers.retain(|v| keep(&v.owner));
        part.issues.retain(|i| keep(&i.owner));
        part.team_states.retain(|t| keep(&t.owner));
        let linear_kept =
            |source: &Source| !source.is_linear() || source_owner(source).is_some_and(keep);
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
