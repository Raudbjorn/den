//! Linear Issue rules (spec 0002, "Next Step rules: Linear Issues"). They run
//! only on the Linear Issues of Owners with a Linear Workflow: an Owner without
//! one has none in the Observation.
//!
//! - Assigned → **Implement** (#32), Waiting On the Linear assignee: me when
//!   that is the Linear viewer of the issue's workspace (by Linear user ID,
//!   never matched to a GitHub login).
//! - Unassigned, in one of the Owner's start teams, state type unstarted (or
//!   backlog when the workflow includes it), and in the active cycle when the
//!   workflow asks → **Start** (#33), Waiting On nobody: anyone may pick it
//!   up. Starts order among themselves by [`StartOrder`].
//! - Any other unassigned Linear Issue is not den's to show, so it gets no
//!   Decision at all. The Linear source asks only for those Start offers;
//!   this rule keeps the list right for facts kept from an earlier survey or
//!   read under another config.
//!
//! den doesn't read a Linear Issue's PR attachments yet (#35), so every
//! Decision says, as an unknown, that a PR for it may already exist.

use super::{Attribution, Context, Decision, Explanation, StartOrder, WorkItem};
use crate::domain::{NextStep, StartPolicy, facts::LinearIssue};

const ASSIGNEE_ROLE: &str = "the Linear assignee";
/// Until #35 reads attachments, den can't tell whether a PR carries the work.
pub const ATTACHMENTS_UNREAD: &str =
    "den doesn't read the Linear Issue's PR attachments yet, so a PR for it may already exist";

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .linear
        .issues
        .iter()
        .filter_map(|issue| decide(ctx, issue))
        .collect()
}

fn decide(ctx: &Context<'_>, issue: &LinearIssue) -> Option<Decision> {
    let item = WorkItem {
        id: issue.work_item_id(),
        identifier: Some(issue.identifier.clone()),
        title: issue.title.clone(),
        url: issue.url.clone(),
        updated_at: issue.updated_at,
    };
    let mut other_conditions = vec![format!(
        "state {} ({}), team {}",
        issue.state.name,
        issue.state.kind.label(),
        issue.team.key
    )];
    let unknowns = vec![ATTACHMENTS_UNREAD.to_owned()];
    if let Some(assignee) = &issue.assignee {
        let viewer = ctx.viewers.linear(&issue.id.workspace);
        return Some(Decision::new(
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
        ));
    }
    let start = &ctx.policy(&issue.owner).start;
    if !start.offers(issue) {
        return None;
    }
    if start.active_cycle_only
        && let Some(cycle) = &issue.cycle
    {
        other_conditions.push(format!(
            "this Linear Workflow offers only the active cycle's issues: {}",
            cycle.describe()
        ));
    }
    let mut decision = Decision::new(
        ctx,
        item,
        Some(NextStep::Start),
        Attribution::nobody("nobody: it is unassigned, so anyone may start it"),
        Explanation {
            winning_rule: Some(start_rule(start, issue)),
            other_conditions,
            unknowns,
            ..Explanation::default()
        },
    );
    decision.start_order = Some(StartOrder {
        urgency: StartOrder::urgency(issue.priority),
        outside_active_cycle: !issue.cycle.as_ref().is_some_and(|c| c.active),
        created_at: issue.created_at,
    });
    Some(decision)
}

/// "the Linear Issue is unassigned, in start team TEAM, and not started yet
/// (Todo)".
fn start_rule(start: &StartPolicy, issue: &LinearIssue) -> String {
    let not_started = if start.include_backlog {
        "not started yet (unstarted or backlog, as this Linear Workflow includes the backlog)"
    } else {
        "not started yet"
    };
    format!(
        "the Linear Issue is unassigned, in start team {}, and {not_started}: {}",
        issue.team.key, issue.state.name
    )
}
