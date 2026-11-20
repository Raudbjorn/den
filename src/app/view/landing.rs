//! Landing Groups in the list and the detail (#35): which Linear Issue a Pull
//! Request belongs to, or is possibly for; and a Linear Issue's members, each
//! with its Repo, number, state, own Next Step and Job.

use super::{State, jobs};
use crate::{
    app::view_model::{JobMarker, LandingDetail, LandingLine, RowLanding, SEPARATOR},
    decide::{
        Decision,
        correlation::{Member, MemberStatus},
    },
    domain::WorkItemId,
    handoff::live,
    ordering::same_work_item,
};

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
    let mut lines: Vec<LandingLine> = landing
        .members
        .iter()
        .map(|member| member_line(state, member))
        .collect();
    if lines.is_empty() {
        lines.push(plain("no Pull Request attached in Linear"));
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

/// `acme/api #4 · open · Merge · Live Job`.
fn member_line(state: &State<'_>, member: &Member) -> LandingLine {
    let id = member.pr.id();
    let mut parts = vec![member.reference(), member.status.describe()];
    if let MemberStatus::Open { .. } = member.status {
        parts.push(own_next_step(state, &id));
    }
    let job = live::most_telling(state.job_checks, &id).map(|c| jobs::marker(&c.state));
    if let Some(marker) = job {
        parts.push(job_words(marker).to_owned());
    }
    parts.extend(member.ambiguity());
    LandingLine {
        text: parts.join(&format!(" {SEPARATOR} ")),
        job,
        caution: member.ambiguous() || matches!(member.status, MemberStatus::NotSurveyed(_)),
    }
}

/// The member's own Next Step, from its own Decision.
fn own_next_step(state: &State<'_>, id: &WorkItemId) -> String {
    state
        .all_decisions
        .iter()
        .find(|d| same_work_item(&d.item.id, id))
        .map_or_else(
            || "its Decision isn't shown".to_owned(),
            |d| match d.next_step {
                Some(step) => format!("Next Step {}", step.label()),
                None => "no Next Step".to_owned(),
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
