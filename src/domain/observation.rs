//! The Observation: one survey of the watched Owners, as facts plus coverage.

use super::{
    coverage::Coverage,
    facts::{Branch, Issue, PullRequest, Run},
    ids::{RepoId, same_login},
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// A team, as `org/slug`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TeamRef {
    pub org: String,
    pub slug: String,
}

/// Who is looking: Waiting On "me" means this login, or a team it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewer {
    pub login: String,
    pub teams: Vec<TeamRef>,
}

impl Viewer {
    pub fn is(&self, login: &str) -> bool {
        same_login(&self.login, login)
    }

    pub fn belongs_to(&self, team: &TeamRef) -> bool {
        self.teams
            .iter()
            .any(|t| same_login(&t.org, &team.org) && t.slug.eq_ignore_ascii_case(&team.slug))
    }
}

/// A Repo of a watched Owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repo {
    pub id: RepoId,
    pub url: String,
    pub is_fork: bool,
    pub is_archived: bool,
    /// `None` for an empty Repo.
    pub default_branch: Option<String>,
    /// The fork's Upstream, when it is a fork whose parent is visible.
    pub parent: Option<RepoId>,
}

/// One survey. Each Work Item kind has its own slot; a slot is empty either
/// because there is nothing, or because it was not read: `coverage` says which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub observed_at: Timestamp,
    pub viewer: Viewer,
    /// The Owners this survey covered, in config order.
    pub owners: Vec<String>,
    pub repos: Vec<Repo>,
    pub pull_requests: Vec<PullRequest>,
    pub issues: Vec<Issue>,
    pub branches: Vec<Branch>,
    pub runs: Vec<Run>,
    pub coverage: Coverage,
}

impl Observation {
    /// An Observation with nothing in it yet.
    pub fn empty(observed_at: Timestamp, viewer: Viewer, owners: Vec<String>) -> Self {
        Self {
            observed_at,
            viewer,
            owners,
            repos: Vec::new(),
            pull_requests: Vec::new(),
            issues: Vec::new(),
            branches: Vec::new(),
            runs: Vec::new(),
            coverage: Coverage::default(),
        }
    }

    pub fn repo(&self, id: &RepoId) -> Option<&Repo> {
        self.repos.iter().find(|r| &r.id == id)
    }
}
