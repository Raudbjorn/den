//! Landing Groups in the list and the detail (#35): which Linear Issue a Pull
//! Request belongs to, or is possibly for; and a Linear Issue's members, each
//! with its Repo, number, state, own Next Step and Job.

use super::{State, detail::linear_issue, jobs};
use crate::{
    app::view_model::{JobMarker, LandingDetail, LandingLine, RowLanding, SEPARATOR},
    decide::{
        Decision,
        correlation::{Member, MemberStatus, PullRequestKey},
    },
    domain::{WorkItemId, facts::AttachmentsRead},
    handoff::live,
};
use std::collections::HashMap;

const LABEL: &str = "LANDING GROUP";

/// A Pull Request row's groups and hints.
pub fn row(decision: &Decision) -> RowLanding {
    let landing = &decision.landing;
    RowLanding {
        groups: landing
            .groups
            .iter()
            .map(|g| g.identifier.clone())
            .collect(),
        ambiguous: landing.ambiguous(),
        possibly_for: landing
            .possibly_for
            .iter()
            .map(|g| g.identifier.clone())
            .collect(),
    }
}

/// The detail's section: a Linear Issue's group, or a Pull Request's groups.
pub fn detail(state: &State<'_>, decision: &Decision) -> Option<LandingDetail> {
    match &decision.item.id {
        WorkItemId::LinearIssue { .. } => Some(issue_group(state, decision)),
        WorkItemId::PullRequest { .. } if !decision.landing.is_empty() => {
            Some(pull_request_groups(decision))
        }
        _ => None,
    }
}

fn issue_group(state: &State<'_>, decision: &Decision) -> LandingDetail {
    let landing = &decision.landing;
    let decisions = pull_request_decisions(state.all_decisions, &landing.members);
    let mut lines: Vec<LandingLine> = landing
        .members
        .iter()
        .map(|member| member_line(state, &decisions, member))
        .collect();
    // den never reads a partial attachment list as "no Pull Request", so
    // the section agrees with the Explanation's "couldn't establish whether
    // a PR exists".
    match unread_attachments(state, decision) {
        Some(why) if lines.is_empty() => lines.push(caution(&format!(
            "den couldn't read all of its attachments in Linear ({why}): it can't tell whether a Pull Request is attached"
        ))),
        Some(why) => lines.push(caution(&format!(
            "den couldn't read all of its attachments in Linear ({why}): more Pull Requests may be attached"
        ))),
        None if lines.is_empty() => lines.push(plain("no Pull Request attached in Linear")),
        None => {}
    }
    let identifier = decision.item.reference();
    lines.extend(landing.hints.iter().map(|hint| {
        plain(&format!(
            "possibly for {identifier}: {} (branch {}), not a member",
            hint.reference(),
            hint.branch
        ))
    }));
    LandingDetail {
        label: LABEL.to_owned(),
        lines,
    }
}

/// Why den didn't read every attachment of the decided Linear Issue, if it
/// didn't. An issue missing from the Observation is unread too: den can't
/// say its list is whole.
fn unread_attachments(state: &State<'_>, decision: &Decision) -> Option<String> {
    match linear_issue(state, &decision.item.id).map(|issue| &issue.attachments.read) {
        Some(AttachmentsRead::Complete) => None,
        Some(AttachmentsRead::Incomplete { why }) => Some(why.clone()),
        None => Some("den has no record of them".to_owned()),
    }
}

/// The members' own Decisions, by Pull Request: one pass over the
/// Decisions per frame, however many members there are. The first one wins.
fn pull_request_decisions<'d>(
    all: &'d [Decision],
    members: &[Member],
) -> HashMap<PullRequestKey, &'d Decision> {
    let mut decisions = HashMap::new();
    if members.is_empty() {
        return decisions;
    }
    for decision in all {
        if let WorkItemId::PullRequest { repo, number } = &decision.item.id {
            decisions
                .entry(PullRequestKey::of(repo, *number))
                .or_insert(decision);
        }
    }
    decisions
}

/// `acme/api #4 · open · Merge · Live Job`.
fn member_line(
    state: &State<'_>,
    decisions: &HashMap<PullRequestKey, &Decision>,
    member: &Member,
) -> LandingLine {
    let id = member.pr.id();
    let mut parts = vec![member.reference(), member.status.describe()];
    if let MemberStatus::Open { .. } = member.status {
        let own = decisions.get(&PullRequestKey::of(&member.pr.repo, member.pr.number));
        parts.push(own_next_step(own.copied()));
    }
    let job = live::most_telling(state.job_checks, &id).map(|c| jobs::marker(&c.state));
    if let Some(marker) = job {
        parts.push(job_words(marker).to_owned());
    }
    parts.extend(member.ambiguity());
    LandingLine {
        text: parts.join(&format!(" {SEPARATOR} ")),
        job,
        caution: member.ambiguous()
            || matches!(
                member.status,
                MemberStatus::NotSurveyed(_) | MemberStatus::OpenAgain
            ),
    }
}

/// The member's own Next Step, from its own Decision.
fn own_next_step(own: Option<&Decision>) -> String {
    own.map_or_else(
        || "its Decision isn't shown".to_owned(),
        |d| match (d.next_step, &d.held_back) {
            (Some(step), _) => format!("Next Step {}", step.label()),
            // As the list and the detail say it: "lands with TEAM-1", "held
            // for release" (#36, #37).
            (None, Some(held)) => held.label(),
            (None, None) => "no Next Step".to_owned(),
        },
    )
}

fn job_words(marker: JobMarker) -> &'static str {
    match marker {
        JobMarker::Live => "Live Job",
        JobMarker::Missing => "Job's worktree gone",
        JobMarker::Unknown => "Job's worktree unknown",
    }
}

fn pull_request_groups(decision: &Decision) -> LandingDetail {
    let landing = &decision.landing;
    let mut lines: Vec<LandingLine> = landing
        .groups
        .iter()
        .map(|g| {
            plain(&format!(
                "member of {}'s Landing Group: attached in Linear",
                g.identifier
            ))
        })
        .collect();
    if landing.ambiguous() {
        let all: Vec<&str> = landing
            .groups
            .iter()
            .map(|g| g.identifier.as_str())
            .collect();
        lines.push(LandingLine {
            text: format!(
                "ambiguous: attached to {} Linear Issues ({})",
                all.len(),
                all.join(", ")
            ),
            job: None,
            caution: true,
        });
    }
    lines.extend(landing.possibly_for.iter().map(|g| {
        plain(&format!(
            "possibly for {}: its branch name names it; a hint, not a member",
            g.identifier
        ))
    }));
    LandingDetail {
        label: LABEL.to_owned(),
        lines,
    }
}

fn plain(text: &str) -> LandingLine {
    LandingLine {
        text: text.to_owned(),
        job: None,
        caution: false,
    }
}

fn caution(text: &str) -> LandingLine {
    LandingLine {
        caution: true,
        ..plain(text)
    }
}
