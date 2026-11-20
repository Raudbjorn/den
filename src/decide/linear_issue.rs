//! Linear Issue rules (spec 0002, "Next Step rules: Linear Issues"). They run
//! only on the Linear Issues of Owners with a Linear Workflow: an Owner without
//! one has none in the Observation.
//!
//! The tracer (#32) knows one rule:
//!
//! - Assigned → **Implement**, Waiting On the Linear assignee: me when that is
//!   the Linear viewer of the issue's workspace (by Linear user ID, never
//!   matched to a GitHub login).
//! - Unassigned → no Next Step yet (Start arrives with the start teams, #33).
//!
//! den doesn't read a Linear Issue's PR attachments yet (#35), so every
//! Decision says, as an unknown, that a PR for it may already exist.

use super::{Attribution, Context, Decision, Explanation, WorkItem};
use crate::domain::{NextStep, facts::LinearIssue};

const ASSIGNEE_ROLE: &str = "the Linear assignee";
/// Until #35 reads attachments, den can't tell whether a PR carries the work.
pub const ATTACHMENTS_UNREAD: &str =
    "den doesn't read the Linear Issue's PR attachments yet, so a PR for it may already exist";

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .linear
        .issues
        .iter()
        .map(|issue| decide(ctx, issue))
        .collect()
}

fn decide(ctx: &Context<'_>, issue: &LinearIssue) -> Decision {
    let item = WorkItem {
        id: issue.work_item_id(),
        identifier: Some(issue.identifier.clone()),
        title: issue.title.clone(),
        url: issue.url.clone(),
        updated_at: issue.updated_at,
    };
    let other_conditions = vec![format!(
        "state {} ({}), team {}",
        issue.state.name,
        issue.state.kind.label(),
        issue.team.key
    )];
    let unknowns = vec![ATTACHMENTS_UNREAD.to_owned()];
    let viewer = ctx.viewers.linear(&issue.id.workspace);
    match &issue.assignee {
        Some(assignee) => Decision::new(
            ctx,
            item,
            Some(NextStep::Implement),
            Attribution::linear_person(viewer, &issue.id.workspace, Some(assignee), ASSIGNEE_ROLE),
            Explanation {
                winning_rule: Some(format!("the Linear Issue is assigned to {}", assignee.name)),
                other_conditions,
                unknowns,
                ..Explanation::default()
            },
        ),
        None => Decision::new(
            ctx,
            item,
            None,
            Attribution::nobody("nobody is assigned"),
            Explanation {
                no_step_reason: Some("the Linear Issue is unassigned".to_owned()),
                other_conditions,
                unknowns,
                ..Explanation::default()
            },
        ),
    }
}
