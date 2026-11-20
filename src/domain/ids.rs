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

    /// The same Repo on GitHub, whose Owner and Repo names are case-insensitive:
    /// facts kept from an earlier survey may spell them as the config did then.
    pub fn same(&self, other: &RepoId) -> bool {
        same_login(&self.owner, &other.owner) && self.name.eq_ignore_ascii_case(&other.name)
    }

    /// The Repo's case-insensitive identity: Owner and Repo names lowercased,
    /// so two spellings of one Repo are one key (the cache's merge, Job dedup).
    #[must_use]
    pub fn folded(&self) -> Self {
        Self::new(
            self.owner.to_ascii_lowercase(),
            self.name.to_ascii_lowercase(),
        )
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

    /// Whether `other` names the same Work Item: the Repo compared as GitHub
    /// does (case-insensitively, [`RepoId::same`]), the rest exactly (Branch
    /// names and workflow paths are case-sensitive in git).
    pub fn same(&self, other: &WorkItemId) -> bool {
        let rest_matches = match (self, other) {
            (Self::Issue { number: x, .. }, Self::Issue { number: y, .. })
            | (Self::PullRequest { number: x, .. }, Self::PullRequest { number: y, .. }) => x == y,
            (Self::Branch { name: x, .. }, Self::Branch { name: y, .. }) => x == y,
            (Self::Run { workflow: x, .. }, Self::Run { workflow: y, .. }) => x == y,
            _ => false,
        };
        rest_matches && self.repo().same(other.repo())
    }

    /// The Work Item's case-insensitive identity: its Repo [folded](RepoId::folded),
    /// Branch and workflow names as they are. Two ids are [`WorkItemId::same`]
    /// exactly when their folded forms are equal.
    #[must_use]
    pub fn folded(&self) -> Self {
        match self {
            Self::Issue { repo, number } => Self::Issue {
                repo: repo.folded(),
                number: *number,
            },
            Self::PullRequest { repo, number } => Self::PullRequest {
                repo: repo.folded(),
                number: *number,
            },
            Self::Branch { repo, name } => Self::Branch {
                repo: repo.folded(),
                name: name.clone(),
            },
            Self::Run { repo, workflow } => Self::Run {
                repo: repo.folded(),
                workflow: workflow.clone(),
            },
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
