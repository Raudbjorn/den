//! Each Owner's Linear Workflow policy, as data for the decision engine (spec
//! 0002, "Product boundaries": the generic rules never branch on an Owner's
//! name; they take the policy).
//!
//! Milestone-2 groundwork (#31): the policy has no rules yet, and every Owner
//! has the empty one. An empty policy changes nothing: an Owner without a
//! Linear Workflow is decided exactly as in milestone 1
//! (`tests/decide_policies.rs`).

use super::ids::same_login;

/// One Owner's Linear Workflow policy. Its [`Default`] is the **empty** policy,
/// the one every Owner without a Linear Workflow gets, and it must stay a
/// policy that changes no Decision.
///
/// Fields arrive with the tickets that use them: the Held Repos (#36), the
/// two switches, exact-head approval and comment reviews need re-review (#36),
/// the configured reviewers (#36) and the landing order (#40). A switch the
/// config turns on by default is still off in the empty policy: the config's
/// defaults are the config's (#32), not this type's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinearWorkflowPolicy {}

/// The Linear Workflow policy of each Owner that has one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policies {
    /// Owner (as configured) and its policy, in config order.
    by_owner: Vec<(String, LinearWorkflowPolicy)>,
}

/// The policy of every Owner without a Linear Workflow.
static EMPTY_POLICY: LinearWorkflowPolicy = LinearWorkflowPolicy {};

impl Policies {
    /// `owner`'s policy is `policy`. Owners compare case-insensitively, as
    /// GitHub logins do: an Owner that already has one keeps its place in
    /// config order, with this spelling and policy.
    #[must_use]
    pub fn with(mut self, owner: impl Into<String>, policy: LinearWorkflowPolicy) -> Self {
        let owner = owner.into();
        match self.by_owner.iter_mut().find(|(o, _)| same_login(o, &owner)) {
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
