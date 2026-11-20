//! Linear Issue rules (spec 0002, "Next Step rules: Linear Issues"). They run
//! only on the Linear Issues of Owners with a Linear Workflow: an Owner without
//! one has none in the Observation.
//!
//! First, the Landing Group (`correlation`, #35). Only **attached** Pull
//! Requests are members; a **hint** (a branch name) never is, so it never
//! withholds a step:
//!
//! - open members → no Next Step: its Pull Requests carry the work;
//! - attachments not read completely → no Next Step: den couldn't establish
//!   whether a Pull Request exists;
//! - a member den can't see → no Next Step, for the same reason;
//! - members, none open → no Next Step (Mark for release, #37, goes here).
//!
//! Then, with no members and every attachment read:
//!
//! - assigned → **Implement**, Waiting On the Linear assignee: me when that is
//!   the Linear viewer of the issue's workspace (by Linear user ID, never
//!   matched to a GitHub login);
//! - unassigned → no Next Step yet (Start arrives with the start teams, #33).

use super::{
    Attribution, Context, Decision, Explanation, WorkItem,
    correlation::{Landing, Member, MemberStatus},
};
use crate::domain::{
    NextStep,
    facts::{Attachment, AttachmentsRead, LinearIssue},
};

const ASSIGNEE_ROLE: &str = "the Linear assignee";
/// The unknown that withholds Start and Implement (spec 0002, rule 4).
pub const PR_EXISTENCE_UNKNOWN: &str = "couldn't establish whether a PR exists";

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
    let landing = ctx.correlation.for_issue(&issue.id);
    let mut other_conditions = vec![format!(
        "state {} ({}), team {}",
        issue.state.name,
        issue.state.kind.label(),
        issue.team.key
    )];
    other_conditions.extend(group_conditions(issue, &landing));
    let unknowns = unknowns(issue, &landing);
    let mut decision = match group_gate(issue, &landing) {
        Some(no_step_reason) => Decision::new(
            ctx,
            item,
            None,
            Attribution::nobody("no Next Step of its own"),
            Explanation {
                no_step_reason: Some(no_step_reason),
                other_conditions,
                unknowns,
                ..Explanation::default()
            },
        ),
        None => by_assignment(ctx, issue, item, other_conditions, unknowns),
    };
    decision.landing = landing;
    decision
}

/// Why the Landing Group leaves the issue no Next Step of its own, if it
/// does (see the module docs, in order).
fn group_gate(issue: &LinearIssue, landing: &Landing) -> Option<String> {
    let open: Vec<&Member> = landing
        .members
        .iter()
        .filter(|m| matches!(m.status, MemberStatus::Open { .. }))
        .collect();
    if !open.is_empty() {
        return Some(format!(
            "its Pull Request{} carr{} the work: {}",
            plural(open.len()),
            if open.len() == 1 { "ies" } else { "y" },
            references(&open)
        ));
    }
    if let AttachmentsRead::Incomplete { .. } = issue.attachments.read {
        return Some(format!(
            "den {PR_EXISTENCE_UNKNOWN}: it couldn't read all of the Linear Issue's attachments"
        ));
    }
    let unseen: Vec<&Member> = landing
        .members
        .iter()
        .filter(|m| matches!(m.status, MemberStatus::NotSurveyed(_)))
        .collect();
    if !unseen.is_empty() {
        return Some(format!(
            "den {PR_EXISTENCE_UNKNOWN}: attached {} (member not surveyed)",
            references(&unseen)
        ));
    }
    if !landing.members.is_empty() {
        let all: Vec<&Member> = landing.members.iter().collect();
        return Some(format!(
            "no open Pull Request is attached: {} (merged or closed)",
            references(&all)
        ));
    }
    None
}

/// No members and every attachment read: the assignment decides.
fn by_assignment(
    ctx: &Context<'_>,
    issue: &LinearIssue,
    item: WorkItem,
    other_conditions: Vec<String>,
    unknowns: Vec<String>,
) -> Decision {
    let viewer = ctx.viewers.linear(&issue.id.workspace);
    match &issue.assignee {
        Some(assignee) => Decision::new(
            ctx,
            item,
            Some(NextStep::Implement),
            Attribution::linear_person(viewer, &issue.id.workspace, Some(assignee), ASSIGNEE_ROLE),
            Explanation {
                winning_rule: Some(format!(
                    "the Linear Issue is assigned to {} and no Pull Request is attached to it",
                    assignee.name
                )),
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

/// The group, in the Explanation: each member and its status, each hint,
/// and the attachments that aren't Pull Requests.
fn group_conditions(issue: &LinearIssue, landing: &Landing) -> Vec<String> {
    let mut lines: Vec<String> = landing
        .members
        .iter()
        .map(|member| {
            let mut line = format!(
                "attached {}: {}",
                member.reference(),
                member.status.describe()
            );
            if let Some(ambiguity) = member.ambiguity() {
                line.push_str(&format!("; {ambiguity}"));
            }
            line
        })
        .collect();
    lines.extend(landing.hints.iter().map(|hint| {
        format!(
            "possibly for {}: {} (branch {}); a branch name is a hint, never a member",
            issue.identifier,
            hint.reference(),
            hint.branch
        )
    }));
    let links = issue
        .attachments
        .links
        .iter()
        .filter(|a| matches!(a, Attachment::Link { .. }))
        .count();
    if links > 0 {
        lines.push(format!(
            "{links} other attachment{} (not GitHub Pull Requests)",
            plural(links)
        ));
    }
    lines
}

fn unknowns(issue: &LinearIssue, landing: &Landing) -> Vec<String> {
    let mut unknowns = Vec::new();
    if let AttachmentsRead::Incomplete { why } = &issue.attachments.read {
        unknowns.push(format!(
            "{PR_EXISTENCE_UNKNOWN}: the Linear Issue's attachments weren't read completely ({why})"
        ));
    }
    unknowns.extend(landing.members.iter().filter_map(|m| match &m.status {
        MemberStatus::NotSurveyed(why) => Some(format!(
            "attached {} is a member den can't see: {}",
            m.reference(),
            why.describe()
        )),
        _ => None,
    }));
    unknowns
}

fn references(members: &[&Member]) -> String {
    members
        .iter()
        .map(|m| m.reference())
        .collect::<Vec<_>>()
        .join(", ")
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}
