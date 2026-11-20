//! The Observation: one survey of the watched Owners, as facts plus coverage.

use super::{
    coverage::Coverage,
    facts::{Branch, Issue, MemberPullRequestState, PullRequest, Run},
    ids::{RepoId, WorkItemId, same_login},
    linear::{LinearPart, LinearViewer},
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// A team, as `org/slug`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TeamRef {
    pub org: String,
    pub slug: String,
}

impl TeamRef {
    /// The same team on GitHub, whose organization logins and team slugs are
    /// case-insensitive.
    pub fn same(&self, other: &TeamRef) -> bool {
        same_login(&self.org, &other.org) && self.slug.eq_ignore_ascii_case(&other.slug)
    }
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
        self.teams.iter().any(|t| t.same(team))
    }
}

/// Everyone den decides "me" by: the GitHub viewer, and the Linear viewer of
/// each Linear Workflow's workspace. A Linear viewer is never matched to a
/// GitHub login (spec 0002, "Waiting On").
///
/// It borrows the viewers from where they are recorded (the Observation), so
/// deciding copies nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewers<'a> {
    pub github: &'a Viewer,
    /// One per workspace whose Linear viewer den read.
    pub linear: &'a [LinearViewer],
}

impl<'a> Viewers<'a> {
    /// The viewers an Observation recorded: its GitHub viewer and the Linear
    /// viewers in its Linear part.
    pub fn of(observation: &'a Observation) -> Self {
        Self {
            github: &observation.viewer,
            linear: &observation.linear.viewers,
        }
    }

    /// The Linear viewer of `workspace`, if den read it.
    pub fn linear(&self, workspace: &str) -> Option<&'a LinearViewer> {
        self.linear
            .iter()
            .find(|v| v.workspace.eq_ignore_ascii_case(workspace))
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
    /// The fork's Upstream as GitHub described it, when it is a fork whose
    /// Upstream den can see. Its own `parent` is never read (always `None`).
    /// den ignores it unless the Upstream is chosen as the target (ADR-0003).
    pub parent: Option<Box<Repo>>,
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
    /// What den read from Linear, for Owners with a Linear Workflow. Empty when
    /// no Owner has one; its sources are in `coverage` like GitHub's.
    pub linear: LinearPart,
    /// What GitHub said, after the survey, of the Landing Group members den
    /// didn't find among the open Pull Requests (#37): merged, closed or open
    /// again. Read by number, with their own coverage
    /// (`SourceKind::MemberStates`, per Owner).
    pub member_states: Vec<MemberPullRequestState>,
    pub coverage: Coverage,
}

impl Repo {
    /// The Upstream, when this Repo is a fork whose Upstream den can see.
    pub fn upstream(&self) -> Option<&Repo> {
        self.parent.as_deref().filter(|_| self.is_fork)
    }
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
            linear: LinearPart::default(),
            member_states: Vec::new(),
            coverage: Coverage::default(),
        }
    }

    pub fn repo(&self, id: &RepoId) -> Option<&Repo> {
        self.repos.iter().find(|r| &r.id == id)
    }

    /// When the survey that read `id`'s facts was made: a Linear Issue's is
    /// the Linear survey's (Linear is surveyed apart from GitHub), any other
    /// Work Item's the GitHub survey's. Data kept from an earlier survey is
    /// older still; the cache records that age.
    pub fn observed_at_of(&self, id: &WorkItemId) -> Timestamp {
        match (id, self.linear.observed_at) {
            (WorkItemId::LinearIssue { .. }, Some(linear)) => linear,
            _ => self.observed_at,
        }
    }
}
