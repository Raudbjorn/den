//! Linear Issue rules (spec 0002, "Next Step rules: Linear Issues"). They run
//! only on the Linear Issues of Owners with a Linear Workflow: an Owner without
//! one has none in the Observation.
//!
//! First, an unassigned Linear Issue the Owner's Linear Workflow doesn't offer
//! to Start is not den's to show, so it gets no Decision at all. The Linear
//! source asks only for Start offers; this keeps the list right for facts
//! kept from an earlier survey or read under another config (#33).
//!
//! Then the Landing Group (`correlation`, #35). Only **attached** Pull
//! Requests are members; a **hint** (a branch name) never is, so it never
//! withholds a step. Start and Implement both need every attachment read and
//! no member (spec 0002, rule 3):
//!
//! - open members → no Next Step: its Pull Requests carry the work;
//! - attachments not read completely → no Next Step: den couldn't establish
//!   whether a Pull Request exists;
//! - a member den can't see → no Next Step, for the same reason;
//! - a link that looks like a Pull Request den can't read
//!   (`Attachment::may_be_pull_request`) → no Next Step, for the same reason;
//! - members, none open → no Next Step (Mark for release, #37, goes here).
//!
//! Then, with no members and every attachment read, the assignment decides:
//!
//! - assigned → **Implement** (#32), Waiting On the Linear assignee: me when
//!   that is the Linear viewer of the issue's workspace (by Linear user ID,
//!   never matched to a GitHub login);
//! - unassigned (and so offered: in one of the Owner's start teams, state
//!   type unstarted, or backlog when the workflow includes it, and in the
//!   active cycle when the workflow asks) → **Start** (#33), Waiting On
//!   nobody: anyone may pick it up. Starts order among themselves by
//!   [`StartOrder`].

use super::{
    Attribution, Context, Decision, Explanation, StartOrder, WorkItem,
    correlation::{Landing, Member, MemberStatus},
};
use crate::domain::{
    NextStep, StartPolicy,
    facts::{Attachment, AttachmentsRead, LinearIssue},
};

const ASSIGNEE_ROLE: &str = "the Linear assignee";
/// The unknown that withholds Start and Implement (spec 0002, rule 4).
pub const PR_EXISTENCE_UNKNOWN: &str = "couldn't establish whether a PR exists";
/// The same unknown once an open member shows a Pull Request exists: only
/// whether more are attached is unknown (the detail's wording too).
pub const MORE_MAY_BE_ATTACHED: &str = "more Pull Requests may be attached";

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .linear
        .issues
        .iter()
        .filter_map(|issue| decide(ctx, issue))
        .collect()
}

fn decide(ctx: &Context<'_>, issue: &LinearIssue) -> Option<Decision> {
    let start = &ctx.policy(&issue.owner).start;
    if issue.assignee.is_none() && !start.offers(issue) {
        return None;
    }
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
    Some(decision)
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
    let unread = pull_request_like_links(issue);
    if !unread.is_empty() {
        return Some(format!(
            "den {PR_EXISTENCE_UNKNOWN}: attached {} looks like a Pull Request it can't read",
            unread.join(", ")
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
        None => start(ctx, issue, item, other_conditions, unknowns),
    }
}

/// An unassigned Linear Issue the workflow offers, with no members and every
/// attachment read: **Start**, Waiting On nobody.
fn start(
    ctx: &Context<'_>,
    issue: &LinearIssue,
    item: WorkItem,
    mut other_conditions: Vec<String>,
    unknowns: Vec<String>,
) -> Decision {
    let start = &ctx.policy(&issue.owner).start;
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
    decision
}

/// "the Linear Issue is unassigned, in start team TEAM, not started yet
/// (Todo), and no Pull Request is attached to it".
fn start_rule(start: &StartPolicy, issue: &LinearIssue) -> String {
    let not_started = if start.include_backlog {
        "not started yet (unstarted or backlog, as this Linear Workflow includes the backlog)"
    } else {
        "not started yet"
    };
    format!(
        "the Linear Issue is unassigned, in start team {}, and {not_started}: {}; no Pull Request is attached to it",
        issue.team.key, issue.state.name
    )
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
    // A link that may be a Pull Request is an unknown, not a plain link.
    let links = issue
        .attachments
        .links
        .iter()
        .filter(|a| matches!(a, Attachment::Link { .. }) && !a.may_be_pull_request())
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
    // An open member is a Pull Request that exists; then only whether more
    // are attached is unknown, so the Explanation agrees with the no-step
    // reason ("its Pull Request carries the work").
    let unknown = if landing
        .members
        .iter()
        .any(|m| matches!(m.status, MemberStatus::Open { .. }))
    {
        MORE_MAY_BE_ATTACHED
    } else {
        PR_EXISTENCE_UNKNOWN
    };
    let mut unknowns = Vec::new();
    if let AttachmentsRead::Incomplete { why } = &issue.attachments.read {
        unknowns.push(format!(
            "{unknown}: the Linear Issue's attachments weren't read completely ({why})"
        ));
    }
    unknowns.extend(
        pull_request_like_links(issue).into_iter().map(|url| {
            format!("{unknown}: attached {url} looks like a Pull Request den can't read")
        }),
    );
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

/// The URLs of the attachments that may be Pull Requests den can't read.
pub fn pull_request_like_links(issue: &LinearIssue) -> Vec<&str> {
    issue
        .attachments
        .links
        .iter()
        .filter_map(|a| match a {
            Attachment::Link { url } if a.may_be_pull_request() => Some(url.as_str()),
            _ => None,
        })
        .collect()
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
