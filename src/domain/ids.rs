//! Identity. Every GitHub Work Item is qualified by Owner and Repo, so two Repos
//! with a PR #1 (or a Branch called `main`) never collide. A Linear Issue is
//! identified by its workspace and Linear's permanent ID, never its identifier
//! (`TEAM-123`), which changes when the issue moves to another team.

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

/// The kinds of Work Item (CONTEXT.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WorkItemKind {
    Issue,
    PullRequest,
    Branch,
    Run,
    /// A Linear Issue of an Owner with a Linear Workflow (milestone 2).
    LinearIssue,
}

/// A Linear Issue's identity: the Linear workspace (its URL key, as the
/// config names it) and Linear's permanent ID for the issue. The identifier
/// (`TEAM-123`) is an attribute: an issue moved to another team gets a new
/// identifier and stays the same Linear Issue (spec 0002, story 16).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LinearIssueId {
    pub workspace: String,
    pub id: String,
}

impl LinearIssueId {
    pub fn new(workspace: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            id: id.into(),
        }
    }
}

impl fmt::Display for LinearIssueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.workspace, self.id)
    }
}

/// Which workflow a Run belongs to. Runs are judged per workflow, so the
/// workflow (not the Run ID) is the Work Item's identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkflowId(pub String);

/// A Work Item's identity: stable across refreshes, unique across Owners and Repos.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WorkItemId {
    Issue {
        repo: RepoId,
        number: u64,
    },
    PullRequest {
        repo: RepoId,
        number: u64,
    },
    Branch {
        repo: RepoId,
        name: String,
    },
    Run {
        repo: RepoId,
        workflow: WorkflowId,
    },
    /// A Linear Issue, under the Owner whose Linear Workflow surveyed it. A
    /// workspace belongs to one Owner's Linear Workflow (the config refuses
    /// two), so `issue` alone identifies it; the Owner places it in Scopes and
    /// coverage like a GitHub Work Item's Owner. The identifier (`TEAM-123`)
    /// is not part of it, so an issue moved between teams keeps its identity.
    LinearIssue {
        owner: String,
        issue: LinearIssueId,
    },
}

impl WorkItemId {
    /// The Repo a GitHub Work Item lives in; `None` for a Linear Issue, which
    /// belongs to an Owner but to no Repo.
    pub fn repo(&self) -> Option<&RepoId> {
        match self {
            Self::Issue { repo, .. }
            | Self::PullRequest { repo, .. }
            | Self::Branch { repo, .. }
            | Self::Run { repo, .. } => Some(repo),
            Self::LinearIssue { .. } => None,
        }
    }

    /// The Owner the Work Item belongs to: its Repo's, or the Owner whose
    /// Linear Workflow surveyed a Linear Issue.
    pub fn owner(&self) -> &str {
        match self {
            Self::Issue { repo, .. }
            | Self::PullRequest { repo, .. }
            | Self::Branch { repo, .. }
            | Self::Run { repo, .. } => &repo.owner,
            Self::LinearIssue { owner, .. } => owner,
        }
    }

    /// The Linear Issue's identity, for a Linear Issue.
    pub fn linear_issue(&self) -> Option<&LinearIssueId> {
        match self {
            Self::LinearIssue { issue, .. } => Some(issue),
            _ => None,
        }
    }

    pub fn kind(&self) -> WorkItemKind {
        match self {
            Self::Issue { .. } => WorkItemKind::Issue,
            Self::PullRequest { .. } => WorkItemKind::PullRequest,
            Self::Branch { .. } => WorkItemKind::Branch,
            Self::Run { .. } => WorkItemKind::Run,
            Self::LinearIssue { .. } => WorkItemKind::LinearIssue,
        }
    }

    /// Whether `other` names the same Work Item: the Repo compared as GitHub
    /// does (case-insensitively, [`RepoId::same`]), the rest exactly (Branch
    /// names and workflow paths are case-sensitive in git). A Linear Issue is
    /// the same when its workspace and Linear's ID match and its Owner folds
    /// to the same login.
    pub fn same(&self, other: &WorkItemId) -> bool {
        let rest_matches = match (self, other) {
            (Self::Issue { number: x, .. }, Self::Issue { number: y, .. })
            | (Self::PullRequest { number: x, .. }, Self::PullRequest { number: y, .. }) => x == y,
            (Self::Branch { name: x, .. }, Self::Branch { name: y, .. }) => x == y,
            (Self::Run { workflow: x, .. }, Self::Run { workflow: y, .. }) => x == y,
            // Workspace + Linear's ID; the Owner folds like a login.
            (
                Self::LinearIssue {
                    owner: x_owner,
                    issue: x,
                },
                Self::LinearIssue {
                    owner: y_owner,
                    issue: y,
                },
            ) => return x == y && same_login(x_owner, y_owner),
            _ => false,
        };
        rest_matches
            && match (self.repo(), other.repo()) {
                (Some(x), Some(y)) => x.same(y),
                _ => false,
            }
    }

    /// The Work Item's case-insensitive identity: its Repo [folded](RepoId::folded),
    /// Branch and workflow names as they are, and a Linear Issue's Owner
    /// lowercased (its workspace and ID as they are). Two ids are
    /// [`WorkItemId::same`] exactly when their folded forms are equal.
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
            Self::LinearIssue { owner, issue } => Self::LinearIssue {
                owner: owner.to_ascii_lowercase(),
                issue: issue.clone(),
            },
        }
    }

    /// The short reference shown beside the Repo: `#12`, a branch name, a
    /// workflow. A Linear Issue's identifier is not part of its identity, so
    /// the list takes it from the Work Item (`WorkItem::reference`) instead;
    /// here it falls back to Linear's permanent ID.
    pub fn reference(&self) -> String {
        match self {
            Self::Issue { number, .. } | Self::PullRequest { number, .. } => format!("#{number}"),
            Self::Branch { name, .. } => name.clone(),
            Self::Run { workflow, .. } => workflow.0.clone(),
            Self::LinearIssue { issue, .. } => issue.id.clone(),
        }
    }
}

impl fmt::Display for WorkItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Issue { repo, number } | Self::PullRequest { repo, number } => {
                write!(f, "{repo}#{number}")
            }
            Self::Branch { repo, name } => write!(f, "{repo}:{name}"),
            Self::Run { repo, workflow } => write!(f, "{repo} workflow {}", workflow.0),
            Self::LinearIssue { issue, .. } => write!(f, "Linear {issue}"),
        }
    }
}

/// GitHub logins are case-insensitive.
pub fn same_login(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// GitHub's longest login, in characters.
pub const MAX_LOGIN_CHARS: usize = 39;
/// GitHub's longest Repo name, in characters.
pub const MAX_REPO_NAME_CHARS: usize = 100;

/// A GitHub login: letters, digits and inner hyphens, at most
/// [`MAX_LOGIN_CHARS`] characters. The one check for the config's Owners and
/// reviewers and for the logins in attachment URLs.
pub fn is_valid_login(login: &str) -> bool {
    (1..=MAX_LOGIN_CHARS).contains(&login.chars().count())
        && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !login.starts_with('-')
        && !login.ends_with('-')
}

/// A GitHub Repo name: letters, digits, `.`, `_` and `-`, at most
/// [`MAX_REPO_NAME_CHARS`] characters, and neither `.` nor `..`. The one
/// check for the config's Repo names and for the Repos in attachment URLs.
pub fn is_valid_repo_name(name: &str) -> bool {
    (1..=MAX_REPO_NAME_CHARS).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && name != "."
        && name != ".."
}
