//! Each Owner's Linear Workflow policy, as data for the decision engine (spec
//! 0002, "Product boundaries": the generic rules never branch on an Owner's
//! name; they take the policy).
//!
//! An Owner without a Linear Workflow gets the empty policy, which changes
//! nothing: it is decided exactly as in milestone 1
//! (`tests/decide_policies.rs`). A Linear Workflow's policy says which
//! unassigned Linear Issues are offered to Start ([`StartPolicy`], #33), its
//! review rules ([`ReviewPolicy`], #36), its Held Repos (#36), its release
//! state (#37) and its landing order (#40).

use super::{
    NextStep, RepoId,
    facts::{LinearIssue, LinearStateKind, Reviewer},
    ids::same_login,
};
use std::collections::BTreeSet;

/// One Owner's Linear Workflow policy. Its [`Default`] is the **empty** policy,
/// the one every Owner without a Linear Workflow gets, and it must stay a
/// policy that changes no Decision: it offers no Start, has no review rules,
/// holds no Repo, names no release state and parks nothing.
///
/// Fields arrive with the tickets that use them.
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
    /// The Repos whose Landing Group members land first, in this order
    /// (#40); every other member lands after them. Empty in the empty
    /// policy, where no Linear Issue (and so no Land) exists.
    pub landing_order: Vec<RepoId>,
    /// Linear states that park a Linear Issue (#74): one in them, with no
    /// Landing Group that decides, has no Next Step instead of Implement or
    /// Start. Compared with the issue's state name, ignoring case, like
    /// [`Self::release_state`]. Empty in the empty policy.
    pub parked_states: Vec<String>,
}

impl LinearWorkflowPolicy {
    /// Where `repo` comes in the landing order: its place in
    /// [`Self::landing_order`], or `None` when it isn't listed (it lands
    /// after every listed Repo). Compared as GitHub compares names.
    pub fn landing_rank(&self, repo: &RepoId) -> Option<usize> {
        self.landing_order.iter().position(|first| first.same(repo))
    }

    /// Whether `repo` is a Held Repo (compared as GitHub compares names).
    pub fn holds(&self, repo: &RepoId) -> bool {
        self.held_repos.iter().any(|held| held.same(repo))
    }

    /// Whether a Review Job for this Owner's Pull Requests returns a verdict
    /// only and runs in the approval-required permission mode (#41): it does
    /// when the Owner has a Linear Workflow, whose review rules every Review
    /// Job follows. Every configured Linear Workflow has review rules
    /// (`LinearWorkflow::policy`); the empty policy has none, so an Owner
    /// without a Linear Workflow keeps milestone 1's Review Job.
    pub fn reviews_verdict_only(&self) -> bool {
        self.review.is_some()
    }

    /// Whether `state_name` is one of this workflow's parked states.
    pub fn is_parked_state(&self, state_name: &str) -> bool {
        self.parked_states
            .iter()
            .any(|parked| parked.eq_ignore_ascii_case(state_name))
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
/// The config's defaults follow GitHub's own rules (ADR-0009, #73):
/// exact-head approval and pending requests holding the merge off, comment
/// reviews needing re-review on. A `ReviewPolicy` built here has whatever it
/// is given.
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
    /// **Pending requests hold the merge**: a PR with a pending review request
    /// stays at Review even when approved (spec 0002, story 44). Off: a
    /// pending request leaves a PR at Review only while it isn't approved,
    /// and an unread request no longer keeps Merge away, as GitHub doesn't
    /// hold a merge for either (ADR-0009).
    pub pending_requests_hold_merge: bool,
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

/// How a Repo merges its Pull Requests (`[repos."OWNER/REPO"] merge_method`,
/// #72, spec 0004): Merge and Land Jobs pass its `gh pr merge` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

impl MergeMethod {
    pub const ALL: [Self; 3] = [Self::Squash, Self::Merge, Self::Rebase];

    /// The config's spelling, which is `gh`'s: `squash`, `merge`, `rebase`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Squash => "squash",
            Self::Merge => "merge",
            Self::Rebase => "rebase",
        }
    }

    /// The method spelled `name`, exactly as [`Self::name`] spells it.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.name() == name)
    }

    /// `gh pr merge`'s flag for it: `--squash`, `--merge`, `--rebase`.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Squash => "--squash",
            Self::Merge => "--merge",
            Self::Rebase => "--rebase",
        }
    }
}

/// Each Repo's merge method (#72). A Repo not listed names none: its Jobs use
/// what the repository allows and ask when it allows more than one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeMethods {
    by_repo: Vec<(RepoId, MergeMethod)>,
}

impl MergeMethods {
    /// No Repo names a merge method.
    pub const fn new() -> Self {
        Self {
            by_repo: Vec::new(),
        }
    }

    /// `repo` merges with `method`. Repos compare as GitHub compares names:
    /// a Repo already listed is replaced.
    #[must_use]
    pub fn with(mut self, repo: RepoId, method: MergeMethod) -> Self {
        match self
            .by_repo
            .iter_mut()
            .find(|(listed, _)| listed.same(&repo))
        {
            Some(entry) => *entry = (repo, method),
            None => self.by_repo.push((repo, method)),
        }
        self
    }

    /// `repo`'s merge method, if it names one.
    pub fn for_repo(&self, repo: &RepoId) -> Option<MergeMethod> {
        self.by_repo
            .iter()
            .find(|(listed, _)| listed.same(repo))
            .map(|(_, method)| *method)
    }
}

/// The Next Step kinds each Repo doesn't track (`[repos."OWNER/REPO"] skip`,
/// spec 0003, "Not tracking a flow"). A Repo not listed tracks every kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Skips {
    by_repo: Vec<(RepoId, BTreeSet<NextStep>)>,
}

/// What a Repo without a `skip` list skips: nothing.
static NO_SKIPS: BTreeSet<NextStep> = BTreeSet::new();

impl Skips {
    /// `repo` doesn't track `steps`. Repos compare as GitHub compares names: a
    /// Repo already listed is replaced.
    #[must_use]
    pub fn with(mut self, repo: RepoId, steps: impl IntoIterator<Item = NextStep>) -> Self {
        let steps: BTreeSet<NextStep> = steps.into_iter().collect();
        match self
            .by_repo
            .iter_mut()
            .find(|(listed, _)| listed.same(&repo))
        {
            Some(entry) => *entry = (repo, steps),
            None => self.by_repo.push((repo, steps)),
        }
        self
    }

    /// The kinds `repo` doesn't track; empty when it isn't listed.
    pub fn for_repo(&self, repo: &RepoId) -> &BTreeSet<NextStep> {
        self.by_repo
            .iter()
            .find(|(listed, _)| listed.same(repo))
            .map_or(&NO_SKIPS, |(_, steps)| steps)
    }

    /// Whether `repo` has a `skip` list (compared as GitHub compares names).
    pub fn lists(&self, repo: &RepoId) -> bool {
        self.by_repo.iter().any(|(listed, _)| listed.same(repo))
    }
}

/// The Linear Workflow policy of each Owner that has one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policies {
    /// Owner (as configured) and its policy, in config order.
    by_owner: Vec<(String, LinearWorkflowPolicy)>,
    /// The Next Step kinds each Repo doesn't track (spec 0003).
    skips: Skips,
    /// The Repos with `screenshots = true` (#79).
    screenshot_repos: Vec<RepoId>,
    /// `legacy_marker_prefixes` (#75, ADR-0010): whose `review-notes`
    /// markers set Review Points, besides den's own.
    legacy_marker_prefixes: Vec<String>,
}

/// The policy of every Owner without a Linear Workflow.
static EMPTY_POLICY: LinearWorkflowPolicy = LinearWorkflowPolicy {
    start: StartPolicy::NONE,
    review: None,
    held_repos: Vec::new(),
    release_state: None,
    landing_order: Vec::new(),
    parked_states: Vec::new(),
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

    /// Every Repo's `skip` list (spec 0003).
    #[must_use]
    pub fn with_skips(mut self, skips: Skips) -> Self {
        self.skips = skips;
        self
    }

    /// The Next Step kinds each Repo doesn't track.
    pub fn skips(&self) -> &Skips {
        &self.skips
    }

    /// The Repos with `screenshots = true` (#79).
    #[must_use]
    pub fn with_screenshot_repos(mut self, repos: Vec<RepoId>) -> Self {
        self.screenshot_repos = repos;
        self
    }

    /// Whether `repo` requires screenshots on its Pull Requests (compared as
    /// GitHub compares names).
    pub fn requires_screenshots(&self, repo: &RepoId) -> bool {
        self.screenshot_repos.iter().any(|listed| listed.same(repo))
    }

    /// The legacy marker prefixes whose markers set Review Points (#75).
    #[must_use]
    pub fn with_legacy_marker_prefixes(mut self, prefixes: Vec<String>) -> Self {
        self.legacy_marker_prefixes = prefixes;
        self
    }

    /// The legacy marker prefixes, as configured; empty by default.
    pub fn legacy_marker_prefixes(&self) -> &[String] {
        &self.legacy_marker_prefixes
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
