//! Linear Issue rules (spec 0002, "Next Step rules: Linear Issues"). They run
//! only on the Linear Issues of Owners with a Linear Workflow: an Owner without
//! one has none in the Observation.
//!
//! First, an unassigned Linear Issue the Owner's Linear Workflow doesn't offer
//! to Start is not den's to show, so it gets no Decision at all, unless its
//! Landing Group has attached members. Keeping their group visible lets den
//! read their states and retain the evidence for Land or Mark for release.
//! Start eligibility never withholds a group decision. The Linear
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
//!    queue, no attachment is a link that may be a Pull Request den can't
//!    read, and den read every attachment and every such member's facts
//!    completely → **Land**, Waiting On the Linear assignee. Otherwise no
//!    Next Step: its Pull Requests carry the work, and the Explanation names
//!    each blocking member with its own Next Step or reason, and each held
//!    member as "held; not blocking". A held member's own state never blocks
//!    (its own row keeps its repair steps: failing checks, conflicts,
//!    ambiguity, facts read in part); it blocks only when den can't tell it
//!    isn't merging right now: queued, not surveyed, or reported open again
//!    (ADR-0011). At least one member outside Held Repos must be open for
//!    there to be anything to land.
//! 2. No open members:
//!    - attachments not read completely, a member den can't see, or a link
//!      that looks like a Pull Request den can't read
//!      (`Attachment::may_be_pull_request`) → no Next Step: den couldn't
//!      establish whether a Pull Request exists;
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
//! 3. Then, with no members and every attachment read, a **parked** state
//!    (the workflow's `parked_states`, #74) comes first: no Next Step,
//!    "parked in <state>", Waiting On nobody. A parked issue whose group
//!    decides (Land, Mark for release, or a reason its Pull Requests carry the
//!    work) is decided by its group above, never parked.
//!
//! 4. Otherwise the assignment decides:
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
/// Starts each Mark for release condition that names member PRs as evidence
/// (the Mark for release sheet lists exactly these, #38).
pub const EVIDENCE_PREFIX: &str = "evidence: ";

pub const DEPLOYMENTS_UNSEEN: &str = "den can't see deployments: Mark for release rests only on the Pull Requests having merged, not on the work having shipped";
/// The same unknown once an open member shows a Pull Request exists: only
/// whether more are attached is unknown (the detail's wording too).
pub const MORE_MAY_BE_ATTACHED: &str = "more Pull Requests may be attached";

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
    let offered = issue.assignee.is_some() || ctx.policy(&issue.owner).start.offers(issue);
    let landing = ctx.correlation.for_issue(&issue.id);
    if !offered && landing.members.is_empty() {
        return None;
    }
    let group = group_rules(ctx, issue, &landing, pull_requests);
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
    if !offered {
        other_conditions.push(
            "unassigned and not offered to Start by this Linear Workflow; shown because its Landing Group decides"
                .to_owned(),
        );
    }
    other_conditions.extend(group_conditions(issue, &landing));
    let mut unknowns = unknowns(issue, &landing);
    let mut decision = match group {
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
    // The Owner's member-state read bears only on an issue with a member
    // whose end den reads by number; it never marks the Owner's other
    // issues incomplete.
    if !landing
        .members
        .iter()
        .any(|m| matches!(m.status, MemberStatus::NotOpen | MemberStatus::Closed))
    {
        decision.coverage.retain(|c| !c.source.is_member_states());
    }
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

/// Waiting On the Linear assignee, for Implement, Land and Mark for release:
/// me when that is the Linear viewer of the issue's workspace.
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
    let unread = pull_request_like_links(issue);
    if !unread.is_empty() {
        return Group::no_step(format!(
            "den {PR_EXISTENCE_UNKNOWN}: attached {} looks like a Pull Request it can't read",
            unread.join(", ")
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
    if let Some(selected) = ctx.policies.linear_repo() {
        for member in open {
            if !held(member) && !member.pr.repo.same(selected) {
                blocking.push(format!(
                    "{} is outside the selected Repo {}; Land requires every member",
                    member.pr.repo, selected
                ));
            }
        }
    }
    let mut unknowns: Vec<String> = Vec::new();
    if let AttachmentsRead::Incomplete { why } = &issue.attachments.read {
        blocking.push(format!(
            "den couldn't read all of the Linear Issue's attachments ({why}), so it can't tell every member"
        ));
    }
    // A link that may be a Pull Request den can't read may be a member it
    // can't judge: the group is possibly incomplete, so never ready.
    blocking.extend(pull_request_like_links(issue).into_iter().map(|url| {
        format!(
            "attached {url} looks like a Pull Request den can't read, so it can't tell every member"
        )
    }));
    // Members named as blocking: a held one among them is not also listed as
    // "held; not blocking" (ADR-0011).
    let mut blocking_members: Vec<WorkItemId> = Vec::new();
    for member in &landing.members {
        let own = own_decision(pull_requests, &member.pr.id());
        let reasons = blocking_reasons(ctx, member, held(member), own, &issue.identifier);
        if !reasons.is_empty() {
            blocking.push(format!("{}: {}", member.reference(), reasons.join("; ")));
            blocking_members.push(member.pr.id());
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
        .filter(|m| {
            held(m)
                && !blocking_members
                    .iter()
                    .any(|b| crate::ordering::same_work_item(b, &m.pr.id()))
        })
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
        // Open, but not in the survey: den can't judge it, and can't tell
        // whether it is in a merge queue either, so it blocks even when held
        // (ADR-0011: a held member blocks only when it may be merging now).
        MemberStatus::OpenAgain => {
            reasons.push(OPEN_AGAIN.to_owned());
            return reasons;
        }
        // Not open: it won't be landed, so it doesn't block.
        MemberStatus::NotOpen | MemberStatus::Merged { .. } | MemberStatus::Closed => {
            return reasons;
        }
    }
    // A held member is never landed, so its ambiguity and its own repair
    // state don't block; only the merge queue holds for every member (rule 1:
    // none is queued; ADR-0011).
    if !held && let Some(ambiguity) = member.ambiguity() {
        reasons.push(ambiguity);
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
    let ambiguity = member
        .ambiguity()
        .map(|a| format!("; {a}"))
        .unwrap_or_default();
    format!(
        "held; not blocking: {} (Held Repo {}; {own_step}{ambiguity})",
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
        conditions.push(format!("{EVIDENCE_PREFIX}merged {}", references(&merged)));
    }
    if !closed.is_empty() {
        conditions.push(format!(
            "{EVIDENCE_PREFIX}closed without merging {}",
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
    // A member den asked about either has a state or left its Owner's
    // entry incomplete; so with no state and a complete entry (or none), den
    // hasn't asked about it yet.
    match source {
        Some(entry) if !entry.state.is_complete() => {
            format!("{} {}", entry.source.label(), entry.state.describe())
        }
        Some(_) | None => "den hasn't read its state yet".to_owned(),
    }
}

/// No members and every attachment read: a parked state, then the assignment,
/// decides.
fn by_assignment(
    ctx: &Context<'_>,
    issue: &LinearIssue,
    item: WorkItem,
    other_conditions: Vec<String>,
    unknowns: Vec<String>,
) -> Decision {
    if ctx.policy(&issue.owner).is_parked_state(&issue.state.name) {
        return Decision::new(
            ctx,
            item,
            None,
            Attribution::nobody(format!(
                "nobody: it is parked in {}, so no step is owed until it leaves that state",
                issue.state.name
            )),
            Explanation {
                no_step_reason: Some(format!("parked in {}", issue.state.name)),
                other_conditions,
                unknowns,
                ..Explanation::default()
            },
        );
    }
    match &issue.assignee {
        Some(person) => Decision::new(
            ctx,
            item,
            Some(NextStep::Implement),
            assignee(ctx, issue),
            Explanation {
                winning_rule: Some(format!(
                    "the Linear Issue is assigned to {} and no Pull Request is attached to it",
                    person.name
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
    // reason ("its Pull Request carries the work"). A member reported open
    // again after the survey is open too (`MemberStatus::is_open`, #37).
    let unknown = if landing.members.iter().any(|m| m.status.is_open()) {
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
