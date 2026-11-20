//! Ports: den's I/O behind traits, so the App and its tests never touch the
//! network, git, kitty or the clock directly. Real adapters live in
//! `crate::github` (and later `crate::workspace`, `crate::launcher`); fakes live
//! with the tests (`tests/support/fakes.rs`).

mod perform;

pub use perform::perform;

use crate::domain::Observation;
use jiff::Timestamp;
use std::sync::Arc;

/// What to survey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurveyRequest {
    pub owners: Vec<String>,
}

/// A survey that produced no Observation at all. Per-source trouble (a failed
/// Owner, errors alongside data, rate limits) is coverage, not a failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SurveyFailure {
    #[error(
        "the GitHub CLI (gh) is not installed: install it (https://cli.github.com), then run `gh auth login`"
    )]
    GhNotInstalled,
    #[error("gh is not logged in to GitHub: run `gh auth login` ({detail})")]
    NotLoggedIn { detail: String },
    #[error("could not read who you are on GitHub: {detail}")]
    ViewerUnavailable { detail: String },
    #[error("the survey stopped before it finished: {detail}")]
    Interrupted { detail: String },
}

impl SurveyFailure {
    /// A short headline for the empty state.
    pub fn headline(&self) -> &'static str {
        match self {
            Self::GhNotInstalled => "gh is not installed",
            Self::NotLoggedIn { .. } => "gh is not logged in",
            Self::ViewerUnavailable { .. } => "GitHub could not be read",
            Self::Interrupted { .. } => "The survey stopped",
        }
    }
}

/// Reads GitHub. Read-only by construction (ADR-0002): there is no write method.
pub trait GitHubSource: Send + Sync {
    fn survey(&self, request: &SurveyRequest) -> Result<Observation, SurveyFailure>;
}

/// git and the filesystem: Main Clones, worktrees, refs. Methods arrive with the
/// launch sheet and Job handoff tickets (#11, #12).
pub trait Workspace: Send + Sync {}

/// Opening a kitty window, or the clipboard fallback. Methods arrive with the
/// Job handoff ticket (#12).
pub trait Launcher: Send + Sync {}

/// The current time, for events that report when something finished.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// The placeholder Workspace and Launcher until their tickets give them methods.
pub struct Unimplemented;

impl Workspace for Unimplemented {}
impl Launcher for Unimplemented {}

/// Every port, as the effect runner needs them.
#[derive(Clone)]
pub struct Ports {
    pub github: Arc<dyn GitHubSource>,
    pub workspace: Arc<dyn Workspace>,
    pub launcher: Arc<dyn Launcher>,
    pub clock: Arc<dyn Clock>,
}
