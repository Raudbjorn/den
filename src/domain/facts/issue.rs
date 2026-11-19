//! Issue facts. Not fetched yet: the Issues ticket (#5) adds the query.

use crate::domain::ids::{RepoId, WorkItemId};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub updated_at: Timestamp,
    /// PRs linked by a closing reference or a manual link. Mentions are not links.
    pub linked_prs: Vec<IssueLink>,
}

/// How an Issue and a PR are linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    ClosingReference,
    ManualLink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueLink {
    pub kind: LinkKind,
    pub repo: RepoId,
    pub number: u64,
    pub is_open: bool,
}

impl Issue {
    pub fn id(&self) -> WorkItemId {
        WorkItemId::Issue {
            repo: self.repo.clone(),
            number: self.number,
        }
    }
}
