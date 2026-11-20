//! Run facts (GitHub Actions workflow runs on a default branch). Not fetched yet:
//! the Runs ticket (#7) adds the query.

use crate::domain::ids::{RepoId, WorkItemId, WorkflowId};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    Queued,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub repo: RepoId,
    pub workflow: WorkflowId,
    pub workflow_name: String,
    pub run_id: u64,
    pub attempt: u32,
    pub url: String,
    pub head_commit: String,
    pub event: String,
    /// The triggering actor; `None` when GitHub has none.
    pub actor: Option<String>,
    pub status: RunStatus,
    pub conclusion: Option<RunConclusion>,
    pub on_default_branch: bool,
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
