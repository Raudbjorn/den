//! The cache: the last Observation per viewer, kept on disk so den starts from
//! it instantly, and merged with each refresh by source (spec, "Cache").
//!
//! - [`CachedObservation`]: the Observation den shows, plus the age of any data it
//!   carried over from an earlier survey because the latest one could not read
//!   that source.
//! - [`merge`]: pure. A source the refresh read completely is replaced; a source it
//!   could not read (failed, partial, not fetched) keeps the earlier data, marked
//!   with that data's own age. One error never wipes the list.
//! - [`FileCache`]: the real [`ObservationCache`](crate::ports::ObservationCache),
//!   one versioned JSON file per viewer under `$XDG_STATE_HOME/den/cache/`.
//!
//! The merge is generic over sources: it works from coverage entries and each
//! fact's Repo, so adding a Work Item kind never needs to touch it.

mod file;
mod merge;

pub use file::{CACHE_FORMAT, CACHE_SUBDIR, FileCache, MAX_CACHE_BYTES};
pub use merge::merge;

use crate::domain::{Observation, Source, SourceKind, SourceState};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Data den kept for a source the latest survey could not read completely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Carried {
    pub source: Source,
    /// When the kept data was observed (older than the Observation's own time).
    pub observed_at: Timestamp,
}

/// The Observation den shows: the latest survey, with earlier data kept for the
/// sources it could not read. Its coverage is always the latest survey's, so a
/// kept source still reads as failed or partial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedObservation {
    pub observation: Observation,
    /// One entry per source whose shown data is older than `observation.observed_at`.
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
