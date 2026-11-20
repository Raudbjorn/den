//! Run facts: GitHub Actions workflow runs, as GitHub reported them.
//!
//! The GitHub source keeps, per workflow, only the latest attempt of the latest
//! Run on a Repo's default branch (spec, "Facts per Work Item kind"). Which Run
//! counts and whether it needs Fix CI is decided in `decide/run.rs`, not here.

use crate::domain::ids::{RepoId, WorkItemId, WorkflowId};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Where a Run is in its lifecycle (GitHub's `status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    Requested,
    Queued,
    Pending,
    Waiting,
    InProgress,
    Completed,
    /// A status den does not know, as GitHub spelled it.
    Unknown(String),
}

impl RunStatus {
    /// Parses GitHub's REST spelling (`in_progress`, `completed`, …).
    pub fn from_github(value: &str) -> Self {
        match value {
            "requested" => Self::Requested,
            "queued" => Self::Queued,
            "pending" => Self::Pending,
            "waiting" => Self::Waiting,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            other => Self::Unknown(other.to_owned()),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Requested => "requested".to_owned(),
            Self::Queued => "queued".to_owned(),
            Self::Pending => "pending".to_owned(),
            Self::Waiting => "waiting".to_owned(),
            Self::InProgress => "in progress".to_owned(),
            Self::Completed => "completed".to_owned(),
            Self::Unknown(raw) => format!("in an unknown state ({raw})"),
        }
    }
}

/// How a completed Run ended (GitHub's `conclusion`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunConclusion {
    Success,
    Failure,
    Cancelled,
    Skipped,
    TimedOut,
    ActionRequired,
    Neutral,
    Stale,
    StartupFailure,
    /// A conclusion den does not know, as GitHub spelled it.
    Unknown(String),
}

impl RunConclusion {
    /// Parses GitHub's REST spelling (`failure`, `timed_out`, …).
    pub fn from_github(value: &str) -> Self {
        match value {
            "success" => Self::Success,
            "failure" => Self::Failure,
            "cancelled" => Self::Cancelled,
            "skipped" => Self::Skipped,
            "timed_out" => Self::TimedOut,
            "action_required" => Self::ActionRequired,
            "neutral" => Self::Neutral,
            "stale" => Self::Stale,
            "startup_failure" => Self::StartupFailure,
            other => Self::Unknown(other.to_owned()),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Success => "succeeded".to_owned(),
            Self::Failure => "failed".to_owned(),
            Self::Cancelled => "was cancelled".to_owned(),
            Self::Skipped => "was skipped".to_owned(),
            Self::TimedOut => "timed out".to_owned(),
            Self::ActionRequired => "needs approval to run".to_owned(),
            Self::Neutral => "ended neutral".to_owned(),
            Self::Stale => "went stale".to_owned(),
            Self::StartupFailure => "failed to start".to_owned(),
            Self::Unknown(raw) => format!("ended with an unknown conclusion ({raw})"),
        }
    }
}

/// One attempt of one workflow Run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub repo: RepoId,
    /// The workflow's file path (`.github/workflows/ci.yml`, or GitHub's
    /// `dynamic/…` path for workflows it generates). Stable per Repo.
    pub workflow: WorkflowId,
    /// The name GitHub shows for this Run's workflow.
    pub workflow_name: String,
    pub run_id: u64,
    /// Which attempt this is; re-running a Run keeps its ID and adds an attempt.
    pub attempt: u32,
    pub url: String,
    pub head_commit: String,
    /// The branch the Run ran on (GitHub's `head_branch`); `None` when GitHub has none.
    pub head_branch: Option<String>,
    /// What triggered the Run (`push`, `schedule`, `workflow_dispatch`, …).
    pub event: String,
    /// Who triggered this attempt; `None` when GitHub has none.
    pub triggering_actor: Option<String>,
    /// Who triggered the Run's first attempt; `None` when GitHub has none.
    pub actor: Option<String>,
    pub status: RunStatus,
    /// `None` until the Run completes.
    pub conclusion: Option<RunConclusion>,
    /// Ran on the Repo's own default branch: not a Pull Request's run, and not a
    /// fork's branch that happens to share the default branch's name.
    pub on_default_branch: bool,
    /// When this attempt started (GitHub's `run_started_at`); `None` when not sent.
    pub started_at: Option<Timestamp>,
    /// GitHub's `updated_at`. GitHub moves it long after a Run ends (log and
    /// artifact housekeeping), so it is a fallback for `started_at` only.
    pub updated_at: Timestamp,
}

impl Run {
    pub fn id(&self) -> WorkItemId {
        WorkItemId::Run {
            repo: self.repo.clone(),
            workflow: self.workflow.clone(),
        }
    }
}
