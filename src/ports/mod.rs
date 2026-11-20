//! Ports: den's I/O behind traits, so the App and its tests never touch the
//! network, git, kitty or the clock directly. Real adapters live in
//! `crate::github`, `crate::linear`, `crate::cache`, `crate::workspace`, `crate::launcher`,
//! `crate::jobs` and `crate::upstreams`; fakes live with the tests (`tests/support/fakes.rs`).

mod perform;

pub use perform::perform;

use crate::{
    cache::{CacheError, CachedObservation},
    config::LinearWorkflow,
    domain::{Coverage, LinearPart, Observation, UpstreamTargets},
    handoff::{PathState, RegisteredWorktree, Remote, WindowCommand, WindowMode},
    jobs::{JobRecord, JobStoreError},
    upstreams::UpstreamStoreError,
};
use jiff::Timestamp;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// What to survey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurveyRequest {
    pub owners: Vec<String>,
    /// The forks whose target is their Upstream: their Branches are compared
    /// against the Upstream's default branch (#13).
    pub upstreams: UpstreamTargets,
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
    /// GitHub's rate limit was hit before den knew who you are.
    #[error("GitHub's rate limit was hit {}", reset_text(.resets_at))]
    RateLimited { resets_at: Option<Timestamp> },
}

fn reset_text(resets_at: &Option<Timestamp>) -> String {
    match resets_at {
        Some(at) => format!("until {}", at.strftime("%H:%M UTC")),
        None => "(GitHub did not say when it resets)".to_owned(),
    }
}

impl SurveyFailure {
    /// A short headline for the empty state.
    pub fn headline(&self) -> &'static str {
        match self {
            Self::GhNotInstalled => "gh is not installed",
            Self::NotLoggedIn { .. } => "gh is not logged in",
            Self::ViewerUnavailable { .. } => "GitHub could not be read",
            Self::Interrupted { .. } => "The survey stopped",
            Self::RateLimited { .. } => "GitHub's rate limit was hit",
        }
    }
}

/// Reads GitHub. Read-only by construction (ADR-0002): there is no write method.
pub trait GitHubSource: Send + Sync {
    fn survey(&self, request: &SurveyRequest) -> Result<Observation, SurveyFailure>;
}

/// What to survey in Linear: the Linear Workflows, one per Owner that has one.
/// Never empty when sent: without Linear Workflows den asks Linear nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearSurveyRequest {
    pub workflows: Vec<LinearWorkflow>,
}

/// What one Linear survey read: the Observation's Linear part, and the
/// coverage of its Linear sources (`Source::Owner { kind: SourceKind::Linear(..) }`
/// only). A source that couldn't be read is coverage, never an error: the
/// survey always answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinearSurvey {
    pub part: LinearPart,
    pub coverage: Coverage,
}

/// Reads Linear for the Owners with a Linear Workflow. Read-only: the two
/// Linear writes ADR-0004 allows are a separate port (#38).
pub trait LinearSource: Send + Sync {
    fn survey(&self, request: &LinearSurveyRequest) -> LinearSurvey;
}

/// den's own on-disk cache of the last Observation, per viewer. Read-only
/// against GitHub like everything else; it writes only den's own files.
pub trait ObservationCache: Send + Sync {
    /// The last viewer's cached Observation; `Ok(None)` when there is none yet.
    fn load(&self) -> Result<Option<CachedObservation>, CacheError>;
    /// Replaces the viewer's cached Observation.
    fn store(&self, cached: &CachedObservation) -> Result<(), CacheError>;
}

/// git and the filesystem: Main Clones, worktrees, refs (spec, "Workspace").
/// Every git call has a timeout. Nothing here deletes anything.
pub trait Workspace: Send + Sync {
    /// Git clones under `root` that could be Main Clones: folders whose `.git`
    /// is a directory (not worktrees), at most a few levels deep, not hidden.
    fn find_clones(&self, root: &Path) -> Result<Vec<PathBuf>, WorkspaceError>;
    /// The clone's remotes, with their fetch URLs.
    fn remotes(&self, clone: &Path) -> Result<Vec<Remote>, WorkspaceError>;
    /// Whether `path` exists, and whether den may write in it.
    fn path_state(&self, path: &Path) -> PathState;
    /// The commit `reference` (a full ref name such as `refs/heads/x`) points
    /// at in the clone; `Ok(None)` when it doesn't exist.
    fn resolve(&self, clone: &Path, reference: &str) -> Result<Option<String>, WorkspaceError>;
    /// Refuses reuse unless the branch tracks the selected remote and branch.
    fn check_tracking(
        &self,
        clone: &Path,
        branch: &str,
        remote: &str,
        remote_branch: &str,
    ) -> Result<(), WorkspaceError>;
    /// The commit `branch` points at on `remote` right now (asks the remote,
    /// changes nothing locally); `Ok(None)` when the remote has no such branch.
    fn remote_branch_head(
        &self,
        clone: &Path,
        remote: &str,
        branch: &str,
    ) -> Result<Option<String>, WorkspaceError>;
    /// The worktrees git has registered for the clone, its own checkout first.
    fn worktrees(&self, clone: &Path) -> Result<Vec<RegisteredWorktree>, WorkspaceError>;
    /// The commit PR `number`'s head ref (`refs/pull/<number>/head`) points at
    /// on `remote` right now (asks the remote, changes nothing locally);
    /// `Ok(None)` when the remote has no such ref. Used for a PR whose head
    /// lives in another Repo than the target (#13).
    fn remote_pull_head(
        &self,
        clone: &Path,
        remote: &str,
        number: u64,
    ) -> Result<Option<String>, WorkspaceError>;
    /// Fetches `refspec` from `remote`.
    fn fetch(&self, clone: &Path, remote: &str, refspec: &str) -> Result<(), WorkspaceError>;
    /// Adds a worktree to the clone.
    fn add_worktree(&self, clone: &Path, add: &WorktreeAdd) -> Result<(), WorkspaceError>;
    /// The commit `branch` points at in the Repo at `url` right now, asked
    /// without a clone (for a Repo den is about to clone); `Ok(None)` when it
    /// has no such branch.
    fn url_branch_head(&self, url: &str, branch: &str) -> Result<Option<String>, WorkspaceError>;
    /// The commit PR `number`'s head (`refs/pull/<number>/head`) is at in the
    /// Repo at `url` right now, asked without a clone (a cross-Repo PR head in
    /// a Repo den is about to clone); `Ok(None)` when it has no such ref.
    fn url_pull_head(&self, url: &str, number: u64) -> Result<Option<String>, WorkspaceError>;
    /// Clones the Repo at `url` into `into`, a folder that must not exist yet:
    /// the adapter refuses rather than clone into anything already there. The
    /// clone's remote for `url` is named `remote`, whatever the user's git
    /// config would name it, so the fetch that follows finds it.
    fn clone_repo(&self, url: &str, remote: &str, into: &Path) -> Result<(), WorkspaceError>;
}

/// A worktree to add: where, on which branch, starting how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeAdd {
    pub path: PathBuf,
    pub branch: String,
    pub start: WorktreeStart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeStart {
    /// Create `branch` at `start` on `remote`, tracking it even when the local name differs.
    NewBranchTracking { start: String, remote: String },
    /// Like NewBranchTracking, but a clone just created by this launch may
    /// already have this branch checked out; detach that clone and reuse it.
    ClonedBranchTracking { start: String, remote: String },
    /// Check out the existing local `branch`.
    ExistingBranch,
    /// Create `branch` at `start` tracking nothing: a new `<type>_<slug>`
    /// branch must not track the default branch it starts from (#12), and a
    /// PR ref whose head branch lives in another Repo has nothing to track (#13).
    NewBranch { start: String },
}

/// A Workspace operation that failed: what den tried, and what went wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{operation}: {detail}")]
pub struct WorkspaceError {
    pub operation: String,
    pub detail: String,
}

/// Opening a kitty window, or the clipboard fallback (spec, "Launcher").
pub trait Launcher: Send + Sync {
    /// Whether a window can open (den runs in kitty) or the clipboard is used.
    fn window_mode(&self) -> WindowMode;
    /// Claude Code (`claude`) is on PATH.
    fn claude_available(&self) -> bool;
    /// Opens a kitty OS window in `window.directory` running Claude Code.
    fn open_window(&self, window: &WindowCommand) -> Result<(), LaunchError>;
    /// Puts `text` on the clipboard.
    fn copy_to_clipboard(&self, text: &str) -> Result<(), LaunchError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{operation}: {detail}")]
pub struct LaunchError {
    pub operation: String,
    pub detail: String,
}

/// den's own record of the Jobs it launched (`crate::jobs`).
pub trait JobStore: Send + Sync {
    fn load(&self) -> Result<Vec<JobRecord>, JobStoreError>;
    /// Adds one record; never removes any.
    fn record(&self, job: &JobRecord) -> Result<(), JobStoreError>;
}

/// den's own record of which forks target their Upstream (`crate::upstreams`).
pub trait UpstreamStore: Send + Sync {
    /// The stored choices; empty when none were ever stored.
    fn load(&self) -> Result<UpstreamTargets, UpstreamStoreError>;
    /// Replaces the stored choices.
    fn store(&self, upstreams: &UpstreamTargets) -> Result<(), UpstreamStoreError>;
}

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

/// Every port, as the effect runner needs them.
#[derive(Clone)]
pub struct Ports {
    pub github: Arc<dyn GitHubSource>,
    pub linear: Arc<dyn LinearSource>,
    pub cache: Arc<dyn ObservationCache>,
    pub workspace: Arc<dyn Workspace>,
    pub launcher: Arc<dyn Launcher>,
    pub jobs: Arc<dyn JobStore>,
    pub upstreams: Arc<dyn UpstreamStore>,
    pub clock: Arc<dyn Clock>,
}
