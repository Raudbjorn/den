//! Each Owner's Linear Workflow policy, as data for the decision engine (spec
//! 0002, "Product boundaries": the generic rules never branch on an Owner's
//! name; they take the policy).
//!
//! An Owner without a Linear Workflow gets the empty policy, which changes
//! nothing: it is decided exactly as in milestone 1
//! (`tests/decide_policies.rs`).

use super::{
    RepoId,
    facts::{LinearIssue, LinearStateKind, Reviewer},
    ids::same_login,
};

/// One Owner's Linear Workflow policy. Its [`Default`] is the **empty** policy,
/// the one every Owner without a Linear Workflow gets, and it must stay a
/// policy that changes no Decision: no Start teams, review rules or Held Repos.
///
/// Fields arrive with the tickets that use them; the landing order comes with
/// #40.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinearWorkflowPolicy {
    /// Which unassigned Linear Issues are offered to Start (#33).
    pub start: StartPolicy,
    /// The workflow's review rules for the Owner's Pull Requests (#36).
    /// `None` (the empty policy): milestone 1's PR rules, unchanged.
    pub review: Option<ReviewPolicy>,
    /// Held Repos: a PR here that would Merge has no Next Step and is held
    /// for release instead (#36). Empty in the empty policy.
    pub held_repos: Vec<RepoId>,
}

impl LinearWorkflowPolicy {
    /// Whether `repo` is a Held Repo (compared as GitHub compares names).
    pub fn holds(&self, repo: &RepoId) -> bool {
        self.held_repos.iter().any(|held| held.same(repo))
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
    /// the PR's current head commit counts, and GitHub's own review decision
    /// must still not ask for more (it tightens GitHub's decision, never
    /// replaces it). Off: GitHub's review decision, as in milestone 1.
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
};

impl Policies {
    /// `owner`'s policy is `policy`. Owners compare case-insensitively, as
    /// GitHub logins do: an Owner that already has one keeps its place in
    /// config order, with this spelling and policy.
    #[must_use]
    pub fn with(mut self, owner: impl Into<String>, policy: LinearWorkflowPolicy) -> Self {
        let owner = owner.into();
        match self
            .by_owner
            .iter_mut()
            .find(|(o, _)| same_login(o, &owner))
        {
            Some(entry) => *entry = (owner, policy),
            None => self.by_owner.push((owner, policy)),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Replacing an Owner's policy keeps the Owner where the config put it,
    /// whatever the casing.
    #[test]
    fn a_replaced_owner_keeps_its_place_in_config_order() {
        let policies = Policies::default()
            .with("acme", LinearWorkflowPolicy::default())
            .with("octo", LinearWorkflowPolicy::default())
            .with("ACME", LinearWorkflowPolicy::default());
        let owners: Vec<&str> = policies.by_owner.iter().map(|(o, _)| o.as_str()).collect();
        assert_eq!(owners, ["ACME", "octo"]);
    }

    /// `for_owner` finds the Owner's own policy under any casing, and the
    /// empty one for an Owner without a Linear Workflow. The policy has no
    /// fields yet (#31), so which one is told by where it lives.
    #[test]
    fn an_owners_policy_is_found_under_any_casing() {
        let policies = Policies::default()
            .with("acme", LinearWorkflowPolicy::default())
            .with("octo", LinearWorkflowPolicy::default());
        let acme = &policies.by_owner[0].1;
        let octo = &policies.by_owner[1].1;
        assert!(std::ptr::eq(policies.for_owner("Acme"), acme));
        assert!(std::ptr::eq(policies.for_owner("OCTO"), octo));
        assert!(std::ptr::eq(policies.for_owner("other"), &EMPTY_POLICY));
    }
}
