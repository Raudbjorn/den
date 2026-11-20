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
//! withholds a step. The rules, in order (spec 0002, "Next Step rules:
//! Linear Issues"):
//!
//! 1. Open members (in the survey, or reported **open again** when den read
//!    their state after it) → the group decides (#37). Every open member
//!    outside a Held Repo would Merge (its own Decision "lands with" this
//!    issue; one open again has no Decision yet, so it blocks), none is
//!    ambiguous, no member is unseen ("not surveyed"), none is in a merge
//!    queue, and den read every attachment and every such member's facts
//!    completely → **Land**, Waiting On the Linear assignee. Otherwise no
//!    Next Step: its Pull Requests carry the work, and the Explanation names
//!    each blocking member with its own Next Step or reason, and each held
//!    member as "held; not blocking". A held member never blocks (its own row
//!    keeps its repair steps), but at least one member outside Held Repos
//!    must be open for there to be anything to land.
//! 2. No open members:
//!    - attachments not read completely, or a member den can't see → no
//!      Next Step: den couldn't establish whether a Pull Request exists;
//!    - in the workflow's release state → no Next Step, "release is outside
//!      den", Waiting On nobody and unattributed (den doesn't know who
//!      releases);
//!    - members, none open, at least one **merged** (read by number after
//!      the survey, `Observation.member_states`), and a state that is
//!      neither the release state nor completed or canceled → **Mark for
//!      release**, Waiting On the Linear assignee. Its Explanation lists the
//!      merged and closed members as evidence and says den can't see
//!      deployments. A member den hasn't read the state of is never taken as
//!      merged: with no member known merged, there is no Next Step and the
//!      Explanation says what den couldn't read.
//!
//! The release-state rule also covers an issue with no members at all: an
//! issue already in its release state is never offered Implement.
//!
//! 3. Then, with no members and every attachment read, the assignment decides:
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
    Attribution, Context, Decision, Explanation, HeldBack, StartOrder, WorkItem,
    correlation::{Landing, Member, MemberStatus},
};
use crate::domain::{
    NextStep, Source, SourceKind, StartPolicy, WaitingOn, WorkItemId,
    facts::{Attachment, AttachmentsRead, LinearIssue, LinearStateKind},
    ids::same_login,
};

const ASSIGNEE_ROLE: &str = "the Linear assignee";
/// Why a member reported open again after the survey can't be judged.
const OPEN_AGAIN: &str = "GitHub reported it open again when den read its state after the survey, which didn't list it, so den hasn't read its facts; refresh to see it";
/// The unknown that withholds Start and Implement (spec 0002, rule 4).
pub const PR_EXISTENCE_UNKNOWN: &str = "couldn't establish whether a PR exists";
/// What Mark for release's Explanation says about what it doesn't claim
/// (spec 0002, story 84).
pub const DEPLOYMENTS_UNSEEN: &str = "den can't see deployments: Mark for release rests only on the Pull Requests having merged, not on the work having shipped";

/// Decides every Linear Issue. `pull_requests` are the Pull Requests' own
/// Decisions, already made: Land reads its members' there.
pub fn decide_all(ctx: &Context<'_>, pull_requests: &[Decision]) -> Vec<Decision> {
    ctx.observation
        .linear
        .issues
        .iter()
        .filter_map(|issue| decide(ctx, issue, pull_requests))
        .collect()
}

fn decide(ctx: &Context<'_>, issue: &LinearIssue, pull_requests: &[Decision]) -> Option<Decision> {
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
    let mut unknowns = unknowns(issue, &landing);
    let mut decision = match group_rules(ctx, issue, &landing, pull_requests) {
        Group::Step {
            step,
            rule,
            conditions,
            unknowns: more,
        } => {
            other_conditions.extend(conditions);
            unknowns.extend(more);
            Decision::new(
                ctx,
                item,
                Some(step),
                assignee(ctx, issue),
                Explanation {
                    winning_rule: Some(rule),
                    other_conditions,
                    unknowns,
                    ..Explanation::default()
                },
            )
        }
        Group::NoStep {
            reason,
            conditions,
            unknowns: more,
            attribution,
        } => {
            other_conditions.extend(conditions);
            unknowns.extend(more);
            Decision::new(
                ctx,
                item,
                None,
                attribution,
                Explanation {
                    no_step_reason: Some(reason),
                    other_conditions,
                    unknowns,
                    ..Explanation::default()
                },
            )
        }
        Group::Ungrouped => by_assignment(ctx, issue, item, other_conditions, unknowns),
    };
    decision.landing = landing;
    Some(decision)
}

/// What the Landing Group decides for its Linear Issue.
enum Group {
    /// **Land** or **Mark for release**, Waiting On the Linear assignee.
    Step {
        step: NextStep,
        rule: String,
        conditions: Vec<String>,
        unknowns: Vec<String>,
    },
    /// No Next Step of its own, for this reason.
    NoStep {
        reason: String,
        conditions: Vec<String>,
        unknowns: Vec<String>,
        attribution: Attribution,
    },
    /// No members, and every attachment read: the assignment decides.
    Ungrouped,
}

impl Group {
    /// No Next Step, Waiting On nobody, with nothing more to say.
    fn no_step(reason: String) -> Self {
        Self::NoStep {
            reason,
            conditions: Vec::new(),
            unknowns: Vec::new(),
            attribution: no_step_of_its_own(),
        }
    }
}

fn no_step_of_its_own() -> Attribution {
    Attribution::nobody("no Next Step of its own")
}

fn assignee(ctx: &Context<'_>, issue: &LinearIssue) -> Attribution {
    let viewer = ctx.viewers.linear(&issue.id.workspace);
    Attribution::linear_person(
        viewer,
        &issue.id.workspace,
        issue.assignee.as_ref(),
        ASSIGNEE_ROLE,
    )
}

/// The Landing Group's rules (see the module docs, in order).
fn group_rules(
    ctx: &Context<'_>,
    issue: &LinearIssue,
    landing: &Landing,
    pull_requests: &[Decision],
) -> Group {
    let open: Vec<&Member> = landing
        .members
        .iter()
        .filter(|m| m.status.is_open())
        .collect();
    if !open.is_empty() {
        return land(ctx, issue, landing, &open, pull_requests);
    }
    if let AttachmentsRead::Incomplete { .. } = issue.attachments.read {
        return Group::no_step(format!(
            "den {PR_EXISTENCE_UNKNOWN}: it couldn't read all of the Linear Issue's attachments"
        ));
    }
    let unseen: Vec<&Member> = landing
        .members
        .iter()
        .filter(|m| matches!(m.status, MemberStatus::NotSurveyed(_)))
        .collect();
    if !unseen.is_empty() {
        return Group::no_step(format!(
            "den {PR_EXISTENCE_UNKNOWN}: attached {} (member not surveyed)",
            references(&unseen)
        ));
    }
    if ctx.policy(&issue.owner).is_release_state(&issue.state.name) {
        return Group::NoStep {
            reason: format!("in {}; release is outside den", issue.state.name),
            conditions: Vec::new(),
            unknowns: Vec::new(),
            attribution: Attribution {
                waiting_on: WaitingOn::Nobody,
                reason: "den doesn't know who releases".to_owned(),
                unattributed: true,
            },
        };
    }
    if landing.members.is_empty() {
        return Group::Ungrouped;
    }
    mark_for_release(ctx, issue, landing)
}

/// Rule 1: the issue has open members. **Land**, or no Next Step with every
/// blocking member named.
fn land(
    ctx: &Context<'_>,
    issue: &LinearIssue,
    landing: &Landing,
    open: &[&Member],
    pull_requests: &[Decision],
) -> Group {
    let policy = ctx.policy(&issue.owner);
    let held = |m: &Member| policy.holds(&m.pr.repo);
    let mut blocking: Vec<String> = Vec::new();
    let mut unknowns: Vec<String> = Vec::new();
    if let AttachmentsRead::Incomplete { why } = &issue.attachments.read {
        blocking.push(format!(
            "den couldn't read all of the Linear Issue's attachments ({why}), so it can't tell every member"
        ));
    }
    for member in &landing.members {
        let own = own_decision(pull_requests, &member.pr.id());
        let reasons = blocking_reasons(ctx, member, held(member), own, &issue.identifier);
        if !reasons.is_empty() {
            blocking.push(format!("{}: {}", member.reference(), reasons.join("; ")));
        }
        if matches!(member.status, MemberStatus::Open { .. })
            && !held(member)
            && let Some(own) = own
        {
            unknowns.extend(
                own.explanation
                    .unknowns
                    .iter()
                    .map(|u| format!("{}: {u}", member.reference())),
            );
        }
    }
    let landing_members: Vec<&Member> = open.iter().copied().filter(|m| !held(m)).collect();
    if landing_members.is_empty() {
        blocking.push(
            "every open member is in a Held Repo, so nothing would land (and Mark for release waits for held Pull Requests)"
                .to_owned(),
        );
    }
    let held_members: Vec<String> = open
        .iter()
        .filter(|m| held(m))
        .map(|m| held_line(m, own_decision(pull_requests, &m.pr.id())))
        .collect();
    if blocking.is_empty() {
        let mut rule = format!(
            "every open member outside a Held Repo would Merge and lands with {}: {}; none is ambiguous, not surveyed or queued, and den read every attachment and every member's facts",
            issue.identifier,
            references(&landing_members)
        );
        if !held_members.is_empty() {
            rule.push_str(&format!(
                "; held members are not landed: {}",
                references(&open.iter().copied().filter(|m| held(m)).collect::<Vec<_>>())
            ));
        }
        return Group::Step {
            step: NextStep::Land,
            rule,
            conditions: held_members,
            unknowns,
        };
    }
    let mut conditions: Vec<String> = blocking.iter().map(|b| format!("blocking: {b}")).collect();
    conditions.extend(held_members);
    Group::NoStep {
        reason: format!(
            "its Pull Request{} carr{} the work: {}; the group can't land yet ({} blocking)",
            plural(open.len()),
            if open.len() == 1 { "ies" } else { "y" },
            references(open),
            blocking.len()
        ),
        conditions,
        unknowns,
        attribution: no_step_of_its_own(),
    }
}

/// Why `member` keeps its group from landing; empty when it doesn't.
fn blocking_reasons(
    ctx: &Context<'_>,
    member: &Member,
    held: bool,
    own: Option<&Decision>,
    identifier: &str,
) -> Vec<String> {
    let mut reasons = Vec::new();
    match &member.status {
        MemberStatus::NotSurveyed(why) => {
            reasons.push(format!("member not surveyed ({})", why.describe()));
            return reasons;
        }
        MemberStatus::Open { .. } => {}
        // Open, but not in the survey: den can't judge it. A held one would
        // never be landed, so it doesn't block.
        MemberStatus::OpenAgain => {
            if !held {
                reasons.push(OPEN_AGAIN.to_owned());
            }
            return reasons;
        }
        // Not open: it won't be landed, so it doesn't block.
        MemberStatus::NotOpen | MemberStatus::Merged { .. } | MemberStatus::Closed => {
            return reasons;
        }
    }
    if member.ambiguous() {
        let others: Vec<&str> = member
            .also_attached_to
            .iter()
            .map(|g| g.identifier.as_str())
            .collect();
        reasons.push(format!("ambiguous, also attached to {}", others.join(", ")));
    }
    let queued = ctx
        .observation
        .pull_requests
        .iter()
        .any(|pr| member.pr.is(&pr.repo, pr.number) && pr.in_merge_queue);
    if queued {
        reasons.push("queued: in the merge queue, not merged yet".to_owned());
    }
    if held {
        return reasons;
    }
    let Some(own) = own else {
        reasons.push("den has no Decision for it".to_owned());
        return reasons;
    };
    match (&own.next_step, &own.held_back) {
        (None, Some(HeldBack::LandsWith { identifiers }))
            if identifiers.iter().any(|i| i == identifier) =>
        {
            let unread: Vec<String> = own
                .coverage
                .iter()
                .filter(|c| c.source != Source::Viewer && !c.state.is_complete())
                .map(|c| format!("{} {}", c.source.label(), c.state.describe()))
                .collect();
            if !unread.is_empty() {
                reasons.push(format!(
                    "den didn't read all of its facts: {}",
                    unread.join("; ")
                ));
            }
        }
        (Some(step), _) => reasons.push(format!(
            "its own Next Step is {} ({})",
            step.label(),
            own.explanation.winning_rule.as_deref().unwrap_or_default()
        )),
        (None, _) => reasons.push(format!(
            "no Next Step of its own ({})",
            own.explanation
                .no_step_reason
                .as_deref()
                .unwrap_or_default()
        )),
    }
    reasons
}

/// "held; not blocking: acme/web #6 (Held Repo acme/web; its own Next Step
/// Resolve conflicts)".
fn held_line(member: &Member, own: Option<&Decision>) -> String {
    let own_step = match own {
        None if member.status == MemberStatus::OpenAgain => OPEN_AGAIN.to_owned(),
        Some(d) => match (&d.next_step, &d.held_back) {
            (Some(step), _) => format!("its own Next Step {}", step.label()),
            (None, Some(held)) => held.label(),
            (None, None) => "no Next Step of its own".to_owned(),
        },
        None => "its Decision isn't known".to_owned(),
    };
    format!(
        "held; not blocking: {} (Held Repo {}; {own_step})",
        member.reference(),
        member.pr.repo
    )
}

fn own_decision<'d>(pull_requests: &'d [Decision], id: &WorkItemId) -> Option<&'d Decision> {
    pull_requests
        .iter()
        .find(|d| crate::ordering::same_work_item(&d.item.id, id))
}

/// Rule 2: members, none open, every attachment read, every member seen, not
/// in the release state. **Mark for release** when one really merged.
fn mark_for_release(ctx: &Context<'_>, issue: &LinearIssue, landing: &Landing) -> Group {
    let all: Vec<&Member> = landing.members.iter().collect();
    if matches!(
        issue.state.kind,
        LinearStateKind::Completed | LinearStateKind::Canceled
    ) {
        return Group::no_step(format!(
            "no open Pull Request is attached ({}), and it is {} ({}): nothing is left for den",
            references(&all),
            issue.state.name,
            issue.state.kind.label()
        ));
    }
    let with = |status: fn(&MemberStatus) -> bool| -> Vec<&Member> {
        landing
            .members
            .iter()
            .filter(|m| status(&m.status))
            .collect()
    };
    let merged = with(|s| matches!(s, MemberStatus::Merged { .. }));
    let closed = with(|s| matches!(s, MemberStatus::Closed));
    let unread = with(|s| matches!(s, MemberStatus::NotOpen));
    let mut conditions = Vec::new();
    if !merged.is_empty() {
        conditions.push(format!("evidence: merged {}", references(&merged)));
    }
    if !closed.is_empty() {
        conditions.push(format!(
            "evidence: closed without merging {}",
            references(&closed)
        ));
    }
    let unknowns: Vec<String> = unread
        .iter()
        .map(|m| {
            format!(
                "whether {} merged or was closed without merging: {}",
                m.reference(),
                why_unread(ctx, m)
            )
        })
        .collect();
    if merged.is_empty() {
        let reason = if unread.is_empty() {
            format!(
                "no open Pull Request is attached, and none merged: {} closed without merging",
                references(&closed)
            )
        } else {
            format!(
                "no open Pull Request is attached: {} (merged or closed); den withholds Mark for release until it has read that one of them merged",
                references(&unread)
            )
        };
        return Group::NoStep {
            reason,
            conditions,
            unknowns,
            attribution: no_step_of_its_own(),
        };
    }
    let Some(release) = &ctx.policy(&issue.owner).release_state else {
        return Group::NoStep {
            reason: format!(
                "its Pull Requests have landed ({} merged), but this Linear Workflow names no release state, so den can't offer Mark for release",
                references(&merged)
            ),
            conditions,
            unknowns,
            attribution: no_step_of_its_own(),
        };
    };
    conditions.push(DEPLOYMENTS_UNSEEN.to_owned());
    Group::Step {
        step: NextStep::MarkForRelease,
        rule: format!(
            "no open Pull Request is attached, and {} merged; it is in {} ({}), neither the release state {release} nor completed or canceled",
            references(&merged),
            issue.state.name,
            issue.state.kind.label()
        ),
        conditions,
        unknowns,
    }
}

/// Why den doesn't know how a not-open member ended.
fn why_unread(ctx: &Context<'_>, member: &Member) -> String {
    let source = ctx
        .observation
        .coverage
        .entries()
        .iter()
        .find(|e| match &e.source {
            Source::Owner {
                owner,
                kind: SourceKind::MemberStates,
            } => same_login(owner, &member.pr.repo.owner),
            _ => false,
        });
    match source {
        Some(entry) if !entry.state.is_complete() => {
            format!("{} {}", entry.source.label(), entry.state.describe())
        }
        Some(_) => "GitHub didn't report its state".to_owned(),
        None => "den hasn't read its state yet".to_owned(),
    }
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
