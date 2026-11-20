//! Each Owner's Linear Workflow policy, as data for the decision engine (spec
//! 0002, "Product boundaries": the generic rules never branch on an Owner's
//! name; they take the policy).
//!
//! An Owner without a Linear Workflow gets the empty policy, which changes
//! nothing: it is decided exactly as in milestone 1
//! (`tests/decide_policies.rs`). A Linear Workflow's policy says which
//! unassigned Linear Issues are offered to Start ([`StartPolicy`], #33), its
//! review rules ([`ReviewPolicy`], #36) and its Held Repos (#36).

use super::{
    RepoId,
    facts::{LinearIssue, LinearStateKind, Reviewer},
    ids::same_login,
};

/// One Owner's Linear Workflow policy. Its [`Default`] is the **empty** policy,
/// the one every Owner without a Linear Workflow gets, and it must stay a
/// policy that changes no Decision: it offers no Start, has no review rules,
/// holds no Repo and names no release state.
///
/// Fields arrive with the tickets that use them; the landing order comes with
/// #40.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinearWorkflowPolicy {
    /// Which unassigned Linear Issues are offered to Start (#33). Offers
    /// none in the empty policy.
    pub start: StartPolicy,
    /// The workflow's review rules for the Owner's Pull Requests (#36).
    /// `None` (the empty policy): milestone 1's PR rules, unchanged.
    pub review: Option<ReviewPolicy>,
    /// Held Repos: a PR here that would Merge has no Next Step and is held
    /// for release instead (#36). Empty in the empty policy.
    pub held_repos: Vec<RepoId>,
    /// The Linear state a landed Linear Issue moves to (#37): Mark for
    /// release is offered only when the workflow names one, and an issue
    /// already in it has no Next Step (release is outside den). Compared
    /// with the issue's state name, ignoring case. `None` in the empty
    /// policy.
    pub release_state: Option<String>,
}

impl LinearWorkflowPolicy {
    /// Whether `repo` is a Held Repo (compared as GitHub compares names).
    pub fn holds(&self, repo: &RepoId) -> bool {
        self.held_repos.iter().any(|held| held.same(repo))
    }

    /// Whether `state_name` is this workflow's release state.
    pub fn is_release_state(&self, state_name: &str) -> bool {
        self.release_state
            .as_deref()
            .is_some_and(|release| release.eq_ignore_ascii_case(state_name))
    }
}

/// The review rules of a Linear Workflow (spec 0002, "Next Step rules: PRs of
/// an Owner with a Linear Workflow"). Having one at all adds the Request
/// review step and judges changes requested by the commit they were given on;
/// the two switches tighten approval and comments further.
///
/// The config turns both switches on by default (#32); that default is the
/// config's. A `ReviewPolicy` built here has whatever it is given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewPolicy {
    /// **Exact-head approval**: only a user's (never a bot's) approval given on
    /// the PR's current head commit counts. Off: GitHub's review decision, as
    /// in milestone 1.
    pub exact_head_approval: bool,
    /// **Comment reviews need re-review**: a user whose latest review only
    /// commented, on an older head, is awaiting re-review.
    pub comment_reviews_need_rereview: bool,
    /// The configured reviewers (GitHub logins or teams), suggested on every
    /// Request review.
    pub reviewers: Vec<Reviewer>,
}

/// Which unassigned Linear Issues an Owner's Linear Workflow offers to
/// **Start** (spec 0002, "Next Step rules: Linear Issues", rule 3). Its
/// [`Default`] names no start team, so it offers none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartPolicy {
    /// The start teams' keys (`TEAM` of `TEAM-123`), compared
    /// case-insensitively.
    pub teams: Vec<String>,
    /// Backlog-state issues are offered too, not only unstarted ones.
    pub include_backlog: bool,
    /// Only issues in their team's active cycle are offered.
    pub active_cycle_only: bool,
}

impl StartPolicy {
    /// The policy that offers nothing.
    pub const NONE: Self = Self {
        teams: Vec::new(),
        include_backlog: false,
        active_cycle_only: false,
    };

    /// Whether `team_key` is one of the start teams.
    pub fn is_start_team(&self, team_key: &str) -> bool {
        self.teams.iter().any(|t| t.eq_ignore_ascii_case(team_key))
    }

    /// Whether a state of `kind` counts as not yet started: unstarted, or
    /// backlog when the workflow includes it.
    pub fn offers_state(&self, kind: &LinearStateKind) -> bool {
        match kind {
            LinearStateKind::Unstarted => true,
            LinearStateKind::Backlog => self.include_backlog,
            _ => false,
        }
    }

    /// Whether `issue` may be offered to Start: unassigned, in a start team,
    /// not yet started, and in the active cycle when the workflow asks.
    pub fn offers(&self, issue: &LinearIssue) -> bool {
        issue.assignee.is_none()
            && self.is_start_team(&issue.team.key)
            && self.offers_state(&issue.state.kind)
            && (!self.active_cycle_only || issue.cycle.as_ref().is_some_and(|c| c.active))
    }
}

/// The Linear Workflow policy of each Owner that has one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policies {
    /// Owner (as configured) and its policy, in config order.
    by_owner: Vec<(String, LinearWorkflowPolicy)>,
}

/// The policy of every Owner without a Linear Workflow.
static EMPTY_POLICY: LinearWorkflowPolicy = LinearWorkflowPolicy {
    start: StartPolicy::NONE,
    review: None,
    held_repos: Vec::new(),
    release_state: None,
};

impl Policies {
    /// `owner`'s policy is `policy` (replacing any it had). Owners compare
    /// case-insensitively, as GitHub logins do.
    pub fn with(mut self, owner: impl Into<String>, policy: LinearWorkflowPolicy) -> Self {
        let owner = owner.into();
        self.by_owner.retain(|(o, _)| !same_login(o, &owner));
        self.by_owner.push((owner, policy));
        self
    }

    /// `owner`'s policy; the empty policy when it has no Linear Workflow.
    pub fn for_owner(&self, owner: &str) -> &LinearWorkflowPolicy {
        self.by_owner
            .iter()
            .find(|(o, _)| same_login(o, owner))
            .map_or(&EMPTY_POLICY, |(_, policy)| policy)
    }
}
