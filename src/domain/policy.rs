//! Each Owner's Linear Workflow policy, as data for the decision engine (spec
//! 0002, "Product boundaries": the generic rules never branch on an Owner's
//! name; they take the policy).
//!
//! Milestone-2 groundwork (#31) gave every Owner the empty policy. An empty
//! policy changes nothing: an Owner without a Linear Workflow is decided
//! exactly as in milestone 1 (`tests/decide_policies.rs`). #33 adds which
//! unassigned Linear Issues are offered to Start ([`StartPolicy`]).

use super::{
    facts::{LinearIssue, LinearStateKind},
    ids::same_login,
};

/// One Owner's Linear Workflow policy. Its [`Default`] is the **empty** policy,
/// the one every Owner without a Linear Workflow gets, and it must stay a
/// policy that changes no Decision.
///
/// Fields arrive with the tickets that use them: Start (#33), the Held Repos (#36), the
/// two switches, exact-head approval and comment reviews need re-review (#36),
/// the configured reviewers (#36) and the landing order (#40). A switch the
/// config turns on by default is still off in the empty policy: the config's
/// defaults are the config's (#32), not this type's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinearWorkflowPolicy {
    /// Which unassigned Linear Issues are offered to Start (#33).
    pub start: StartPolicy,
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
