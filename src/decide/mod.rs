//! The decision engine (test seam 1): `decide(Observation, viewer, now) -> Decisions`.
//!
//! Pure: no I/O, no clock reads, deterministic. Each Work Item kind has its own
//! module with its own rules; this module only dispatches and defines what a
//! Decision is. Adding a kind's rules means editing that kind's module only.

mod attribution;
mod branch;
mod issue;
mod pull_request;
mod run;

pub use attribution::Attribution;

use crate::domain::{CoverageEntry, NextStep, Observation, Viewer, WaitingOn, WorkItemId};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// What the list shows about a Work Item, independent of its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: WorkItemId,
    pub title: String,
    pub url: String,
    pub updated_at: Timestamp,
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
        }
    }
}

/// Everything a kind's rules may look at.
pub struct Context<'a> {
    pub observation: &'a Observation,
    pub viewer: &'a Viewer,
    pub now: Timestamp,
}

/// Decides every Work Item in the Observation, in kind order (Pull Requests,
/// Issues, Branches, Runs) and source order within a kind. Sorting for display
/// is `crate::ordering`'s job.
pub fn decide(observation: &Observation, viewer: &Viewer, now: Timestamp) -> Vec<Decision> {
    let ctx = Context {
        observation,
        viewer,
        now,
    };
    let mut decisions = pull_request::decide_all(&ctx);
    decisions.extend(issue::decide_all(&ctx));
    decisions.extend(branch::decide_all(&ctx));
    decisions.extend(run::decide_all(&ctx));
    decisions
}
