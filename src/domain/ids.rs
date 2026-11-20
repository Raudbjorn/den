//! Identity. Every Work Item is qualified by Owner and Repo, so two Repos with a
//! PR #1 (or a Branch called `main`) never collide.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A Repo, as `owner/name`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RepoId {
    pub owner: String,
    pub name: String,
}

impl RepoId {
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            name: name.into(),
        }
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// The four kinds of Work Item (CONTEXT.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WorkItemKind {
    Issue,
    PullRequest,
    Branch,
    Run,
}

/// Which workflow a Run belongs to. Runs are judged per workflow, so the
/// workflow (not the Run ID) is the Work Item's identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkflowId(pub String);

/// A Work Item's identity: stable across refreshes, unique across Owners and Repos.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WorkItemId {
    Issue { repo: RepoId, number: u64 },
    PullRequest { repo: RepoId, number: u64 },
    Branch { repo: RepoId, name: String },
    Run { repo: RepoId, workflow: WorkflowId },
}

impl WorkItemId {
    pub fn repo(&self) -> &RepoId {
        match self {
            Self::Issue { repo, .. }
            | Self::PullRequest { repo, .. }
            | Self::Branch { repo, .. }
            | Self::Run { repo, .. } => repo,
        }
    }

    pub fn kind(&self) -> WorkItemKind {
        match self {
            Self::Issue { .. } => WorkItemKind::Issue,
            Self::PullRequest { .. } => WorkItemKind::PullRequest,
            Self::Branch { .. } => WorkItemKind::Branch,
            Self::Run { .. } => WorkItemKind::Run,
        }
    }

    /// The short reference shown beside the Repo: `#12`, a branch name, a workflow.
    pub fn reference(&self) -> String {
        match self {
            Self::Issue { number, .. } | Self::PullRequest { number, .. } => format!("#{number}"),
            Self::Branch { name, .. } => name.clone(),
            Self::Run { workflow, .. } => workflow.0.clone(),
        }
    }
}

impl fmt::Display for WorkItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.repo(),
            match self {
                Self::Issue { number, .. } | Self::PullRequest { number, .. } =>
                    format!("#{number}"),
                Self::Branch { name, .. } => format!(":{name}"),
                Self::Run { workflow, .. } => format!(" workflow {}", workflow.0),
            }
        )
    }
}

/// GitHub logins are case-insensitive.
pub fn same_login(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
