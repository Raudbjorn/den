//! Issue facts: what GitHub said about an open Issue and the PRs linked to it.

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
    /// False when GitHub left the labels out or some were unreadable, so the
    /// Issue may be labelled although `labels` is empty.
    pub labels_complete: bool,
    pub assignees: Vec<String>,
    /// False when GitHub left the assignees out or some were unreadable, so the
    /// Issue may be assigned although `assignees` is empty.
    pub assignees_complete: bool,
    pub updated_at: Timestamp,
    /// PRs linked by a closing reference or a manual link, in any Repo, open or
    /// not. Mentions in a title, body or branch name are not links.
    pub linked_prs: Vec<IssueLink>,
    /// False when GitHub reported more linked PRs than den read, so an open one
    /// may be missing from `linked_prs`.
    pub linked_prs_complete: bool,
}

/// How an Issue and a PR are linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LinkKind {
    /// The PR says it closes the Issue ("Fixes #12" and the like).
    ClosingReference,
    /// Someone linked the PR to the Issue by hand (the Development sidebar).
    ManualLink,
}

impl LinkKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::ClosingReference => "closing reference",
            Self::ManualLink => "manual link",
        }
    }
}

/// A linked PR's state, as GitHub reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LinkedPrState {
    Open,
    Closed,
    Merged,
}

impl LinkedPrState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueLink {
    pub kind: LinkKind,
    /// The Repo the PR lives in; may be outside the watched Owners.
    pub repo: RepoId,
    pub number: u64,
    pub state: LinkedPrState,
}

impl IssueLink {
    pub fn is_open(&self) -> bool {
        self.state == LinkedPrState::Open
    }
}

impl Issue {
    pub fn id(&self) -> WorkItemId {
        WorkItemId::Issue {
            repo: self.repo.clone(),
            number: self.number,
        }
    }
}
