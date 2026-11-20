//! The detail pane: the selected item's Next Step, Waiting On and Explanation.

use super::{State, coverage, format, jobs, landing};
use crate::{
    app::view_model::{Detail, DetailContent, DetailFacts},
    decide::Decision,
    domain::{WorkItemId, facts::LinearIssue},
};

const LABEL: &str = "D E T A I L";
const PLACEHOLDER: &str = "Nothing selected.";

pub fn build(state: &State<'_>) -> Detail {
    let content = state
        .selected
        .and_then(|i| state.decisions.get(i))
        .map(|decision| {
            let explanation = &decision.explanation;
            let why = explanation
                .winning_rule
                .clone()
                .or_else(|| explanation.no_step_reason.clone())
                .unwrap_or_default();
            DetailContent {
                reference: reference(decision),
                title: decision.item.title.clone(),
                url: decision.item.url.clone(),
                next_step: decision
                    .next_step
                    .map_or("none", |step| step.label())
                    .to_owned(),
                waiting_on: if decision.unattributed {
                    format!("{} (unattributed)", decision.waiting_on.label())
                } else {
                    decision.waiting_on.label().to_owned()
                },
                waiting_reason: decision.waiting_reason.clone(),
                why,
                also: explanation.other_conditions.clone(),
                unknowns: explanation.unknowns.clone(),
                coverage: decision
                    .coverage
                    .iter()
                    .filter(|c| !c.state.is_complete())
                    .map(|c| coverage::entry_line(state, c, decision.item.id.repo()))
                    .collect(),
                observed: observed(
                    state,
                    coverage::item_observed_at(state, &decision.coverage, decision.observed_at),
                ),
                job: jobs::detail(state.job_checks, &decision.item.id),
                facts: linear_issue(state, &decision.item.id).map(linear_facts),
                landing: landing::detail(state, decision),
            }
        });
    Detail {
        label: LABEL.to_owned(),
        page: state.detail_page,
        content,
        placeholder: PLACEHOLDER.to_owned(),
    }
}

/// "12:00 UTC, 4 min ago" (without the age when the App has no time yet).
fn observed(state: &State<'_>, at: jiff::Timestamp) -> String {
    let ago = format::ago(state.now, at);
    if ago.is_empty() {
        format::clock_time(at)
    } else {
        format!("{}, {ago}", format::clock_time(at))
    }
}

/// `owner/name #12`; a Linear Issue's identifier, its workspace and Owner.
fn reference(decision: &Decision) -> String {
    match &decision.item.id {
        WorkItemId::LinearIssue { owner, issue } => format!(
            "{} {} Linear workspace {} {} Owner {owner}",
            decision.item.reference(),
            crate::app::view_model::SEPARATOR,
            issue.workspace,
            crate::app::view_model::SEPARATOR,
        ),
        id => id.to_string(),
    }
}

pub(super) fn linear_issue<'s>(state: &State<'s>, id: &WorkItemId) -> Option<&'s LinearIssue> {
    let issue = id.linear_issue()?;
    state
        .observation?
        .linear
        .issues
        .iter()
        .find(|i| &i.id == issue)
}

/// Spec 0002, story 17: team, state, assignee, priority, cycle (the URL is
/// the detail's own last line).
fn linear_facts(issue: &LinearIssue) -> DetailFacts {
    DetailFacts {
        label: "LINEAR ISSUE".to_owned(),
        lines: vec![
            format!("team {}", issue.team.key),
            format!("state {} ({})", issue.state.name, issue.state.kind.label()),
            match &issue.assignee {
                Some(assignee) => format!("assignee {}", assignee.name),
                None => "unassigned".to_owned(),
            },
            format!("priority {}", issue.priority.label()),
            issue
                .cycle
                .as_ref()
                .map_or_else(|| "no cycle".to_owned(), |c| c.describe()),
        ],
    }
}
