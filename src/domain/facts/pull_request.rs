//! Pull Request facts.
//!
//! The PR rules ticket (#4) adds mergeability, checks (required / advisory),
//! review decision, unresolved threads, requested reviewers and merge-queue state
//! here, alongside the query that fetches them (`github/pull_requests.rs`).

use crate::domain::ids::{RepoId, WorkItemId};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub url: String,
    /// `None` when GitHub has no author (a deleted account).
    pub author: Option<String>,
    pub is_draft: bool,
    pub base_ref: String,
    pub head_ref: String,
    pub head_commit: String,
    /// The Repo the head branch lives in; differs from `repo` for cross-Repo PRs.
    /// `None` when that Repo was deleted.
    pub head_repo: Option<RepoId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl PullRequest {
    pub fn id(&self) -> WorkItemId {
        WorkItemId::PullRequest {
            repo: self.repo.clone(),
            number: self.number,
        }
    }
}
