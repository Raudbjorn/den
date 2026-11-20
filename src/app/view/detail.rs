//! The detail pane: the selected item's Next Step, Waiting On and Explanation.

use super::{State, coverage, format, jobs};
use crate::{
    app::view_model::{Detail, DetailContent, DetailFacts},
    decide::Decision,
    domain::{
        WorkItemId,
        facts::{LinearIssue, Reviewer},
    },
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
                next_step: match (decision.next_step, &decision.held_back) {
                    (Some(step), _) => step.label().to_owned(),
                    (None, Some(held)) => format!("none ({})", held.label()),
                    (None, None) => "none".to_owned(),
                },
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
                facts: linear_issue(state, &decision.item.id)
                    .map(linear_facts)
                    .or_else(|| workflow_facts(decision)),
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

fn linear_issue<'s>(state: &State<'s>, id: &WorkItemId) -> Option<&'s LinearIssue> {
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

/// A PR of an Owner with a Linear Workflow (#36): the hold, if any, and the
/// suggested reviewers (configured plus awaiting re-review, minus fresh
/// approvers, pending requests and the author).
fn workflow_facts(decision: &Decision) -> Option<DetailFacts> {
    let suggested = decision.suggested_reviewers.as_ref()?;
    let mut lines = Vec::new();
    if let Some(held) = &decision.held_back {
        lines.push(held.label());
    }
    lines.push(if suggested.is_empty() {
        "suggested reviewers: none".to_owned()
    } else {
        let names: Vec<String> = suggested
            .iter()
            .map(|reviewer| match reviewer {
                Reviewer::User(login) => login.clone(),
                Reviewer::Team(team) => format!("team {}/{}", team.org, team.slug),
            })
            .collect();
        format!("suggested reviewers: {}", names.join(", "))
    });
    Some(DetailFacts {
        label: "LINEAR WORKFLOW".to_owned(),
        lines,
    })
}
