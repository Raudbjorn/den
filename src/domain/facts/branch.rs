//! Branch facts: what GitHub said about a Branch of a watched Repo. Whether it
//! is a Work Item, and which Next Step it needs, is `decide::branch`'s call.

use crate::domain::ids::{RepoId, WorkItemId};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// The branch a Branch's commits are counted against: the Repo's own default
/// branch (for a fork, the fork's own, never its parent's: ADR-0003), or the
/// Upstream's default branch once an Upstream is set for the fork (#13).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComparisonBranch {
    /// The Repo the comparison branch lives in.
    pub repo: RepoId,
    pub name: String,
}

impl ComparisonBranch {
    /// The comparison branch as shown beside a Branch of `of`: just the name in
    /// the same Repo, `owner/name:branch` in another.
    pub fn describe(&self, of: &RepoId) -> String {
        if &self.repo == of {
            self.name.clone()
        } else {
            format!("{}:{}", self.repo, self.name)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub repo: RepoId,
    pub name: String,
    pub url: String,
    pub last_commit_at: Timestamp,
    /// The last commit's author matched to a GitHub login; `None` when unmatched.
    pub last_commit_author: Option<String>,
    /// The first line of the last commit's message.
    pub last_commit_headline: String,
    /// What `commits_ahead` counts against.
    pub comparison: ComparisonBranch,
    /// Commits on this Branch that are not on the comparison branch. `None` when
    /// den did not compare it (a Branch that was already a PR head is never
    /// compared: it is not a Work Item either way).
    pub commits_ahead: Option<u32>,
    /// Whether any PR (open, merged or closed, in any Repo) ever used this
    /// Branch as its head.
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
