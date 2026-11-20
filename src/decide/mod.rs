//! The decision engine (test seam 1):
//! `decide(Observation, viewers, policies, now) -> Decisions`.
//!
//! Pure: no I/O, no clock reads, deterministic. `viewers` says who "me" is (the
//! GitHub viewer, and a Linear viewer per workspace) and `policies`
//! holds each Owner's Linear Workflow policy, which is empty, and changes
//! nothing, for an Owner without one. Decisions are made on the whole
//! Observation; Scope filtering comes after (`crate::ordering`). Each Work Item kind has its own
//! module with its own rules; this module only dispatches and defines what a
//! Decision is. Adding a kind's rules means editing that kind's module only.

mod attribution;
mod branch;
mod issue;
mod linear_issue;
mod pull_request;
mod run;

pub use attribution::Attribution;

use crate::domain::{
    CoverageEntry, LinearWorkflowPolicy, NextStep, Observation, Policies, UpstreamTargets, Viewers,
    WaitingOn, WorkItemId, facts::LinearPriority,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// What the list shows about a Work Item, independent of its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: WorkItemId,
    /// A Linear Issue's identifier (`TEAM-123`): shown, never compared.
    /// `None` for GitHub Work Items, whose reference comes from their id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    pub title: String,
    pub url: String,
    pub updated_at: Timestamp,
}

impl WorkItem {
    /// The short reference shown beside the Repo or Owner: `#12`, a branch
    /// name, a workflow, a Linear identifier.
    pub fn reference(&self) -> String {
        self.identifier
            .clone()
            .unwrap_or_else(|| self.id.reference())
    }
}

/// Why a Next Step won (or why there is none), what else holds, what is unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Explanation {
    /// The rule that produced the Next Step, e.g. "the PR is a draft".
    pub winning_rule: Option<String>,
    /// Why no rule matched, when there is no Next Step.
    pub no_step_reason: Option<String>,
    /// Other conditions that also hold, e.g. "2 failing checks".
    pub other_conditions: Vec<String>,
    /// Facts den could not establish, e.g. "mergeability not computed yet".
    pub unknowns: Vec<String>,
}

/// The decision for one Work Item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub item: WorkItem,
    pub next_step: Option<NextStep>,
    pub waiting_on: WaitingOn,
    /// Why it is Waiting On whoever it is.
    pub waiting_reason: String,
    /// den could not establish who owes the step; kept visible, Waiting On nobody.
    pub unattributed: bool,
    pub explanation: Explanation,
    /// Coverage of the sources this item's facts came from.
    pub coverage: Vec<CoverageEntry>,
    /// When the facts behind this Decision were observed.
    pub observed_at: Timestamp,
    /// Where a Start falls among Starts (#33); `None` for every other Next
    /// Step, which order by update time alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_order: Option<StartOrder>,
}

/// Where a **Start** falls among Starts (spec 0002, "Ordering": priority, then
/// active cycle, then oldest first). Lower comes first: the fields compare in
/// declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StartOrder {
    /// Linear's priority as urgency: urgent 0, high 1, medium 2, low 3, a
    /// number den doesn't know 4, no priority 5 (see [`StartOrder::urgency`]).
    pub urgency: u8,
    /// `false` when the issue is in its team's active cycle, so those come first.
    pub outside_active_cycle: bool,
    /// When the issue was created: the oldest comes first.
    pub created_at: Timestamp,
}

impl StartOrder {
    /// Linear's priority as an urgency rank (lower is more urgent). Linear's
    /// own number puts "no priority" (0) first; den puts it last.
    pub fn urgency(priority: LinearPriority) -> u8 {
        match priority {
            LinearPriority::Urgent => 0,
            LinearPriority::High => 1,
            LinearPriority::Medium => 2,
            LinearPriority::Low => 3,
            LinearPriority::Other(_) => 4,
            LinearPriority::None => 5,
        }
    }
}

impl Decision {
    /// Builds a Decision, attaching the coverage that applies to the item.
    pub fn new(
        ctx: &Context<'_>,
        item: WorkItem,
        next_step: Option<NextStep>,
        attribution: Attribution,
        explanation: Explanation,
    ) -> Self {
        let coverage = ctx.observation.coverage.applicable_to(&item.id);
        Self {
            item,
            next_step,
            waiting_on: attribution.waiting_on,
            waiting_reason: attribution.reason,
            unattributed: attribution.unattributed,
            explanation,
            coverage,
            observed_at: ctx.observation.observed_at,
            start_order: None,
        }
    }
}

/// Everything a kind's rules may look at.
pub struct Context<'a> {
    pub observation: &'a Observation,
    pub viewers: &'a Viewers<'a>,
    /// Each Owner's Linear Workflow policy; read it through [`Context::policy`].
    pub policies: &'a Policies,
    pub now: Timestamp,
    /// The forks whose target is their Upstream (#13): their Branches must have
    /// been compared against the Upstream's default branch.
    pub upstreams: &'a UpstreamTargets,
}

impl Context<'_> {
    /// The Linear Workflow policy of `owner`: the empty policy, which changes
    /// nothing, when `owner` has no Linear Workflow.
    pub fn policy(&self, owner: &str) -> &LinearWorkflowPolicy {
        self.policies.for_owner(owner)
    }
}

/// Decides every Work Item in the Observation, in kind order (Pull Requests,
/// Issues, Branches, Runs, Linear Issues) and source order within a kind. Sorting for display
/// is `crate::ordering`'s job.
///
/// Every fork is its own target (ADR-0003); [`decide_with`] takes the forks
/// whose target is their Upstream.
pub fn decide(
    observation: &Observation,
    viewers: &Viewers,
    policies: &Policies,
    now: Timestamp,
) -> Vec<Decision> {
    decide_with(
        observation,
        viewers,
        policies,
        &UpstreamTargets::default(),
        now,
    )
}

/// [`decide`], with `upstreams` naming the forks whose target is their Upstream.
pub fn decide_with(
    observation: &Observation,
    viewers: &Viewers,
    policies: &Policies,
    upstreams: &UpstreamTargets,
    now: Timestamp,
) -> Vec<Decision> {
    let ctx = Context {
        observation,
        viewers,
        policies,
        now,
        upstreams,
    };
    let mut decisions = pull_request::decide_all(&ctx);
    decisions.extend(issue::decide_all(&ctx));
    decisions.extend(branch::decide_all(&ctx));
    decisions.extend(run::decide_all(&ctx));
    decisions.extend(linear_issue::decide_all(&ctx));
    decisions
}
