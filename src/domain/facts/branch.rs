//! Branch facts. Not fetched yet: the Branches ticket (#6) adds the query.

use crate::domain::ids::{RepoId, WorkItemId};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub repo: RepoId,
    pub name: String,
    pub url: String,
    pub last_commit_at: Timestamp,
    /// The last commit's author matched to a GitHub login; `None` when unmatched.
    pub last_commit_author: Option<String>,
    /// Commits ahead of the comparison branch (the fork's own default branch, or
    /// Upstream's when set).
    pub commits_ahead: u32,
    /// Whether any PR ever used this branch as its head.
    pub ever_had_pr: bool,
}

impl Branch {
    pub fn id(&self) -> WorkItemId {
        WorkItemId::Branch {
            repo: self.repo.clone(),
            name: self.name.clone(),
        }
    }
}
