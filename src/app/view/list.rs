//! The Next Step list and its empty states.

use super::{State, format, header, jobs, landing, scope};
use crate::cache;
use crate::{
    app::view_model::{ELLIPSIS, EmptyState, List, Row},
    decide::Decision,
    domain::WaitingOn,
    handoff::live,
};

const LABEL: &str = "N E X T   S T E P S";
const NO_STEP: &str = "no step yet";

pub fn build(state: &State<'_>) -> List {
    let rows: Vec<Row> = state.decisions.iter().map(|d| row(state, d)).collect();
    let empty = rows.is_empty().then(|| empty_state(state));
    List {
        label: LABEL.to_owned(),
        rows,
        selected: state.selected,
        empty,
    }
}

fn row(state: &State<'_>, decision: &Decision) -> Row {
    let id = &decision.item.id;
    Row {
        id: id.clone(),
        kind: id.kind(),
        next_step: decision.next_step,
        step_label: decision
            .next_step
            .map_or(NO_STEP, |step| step.label())
            .to_owned(),
        repo: id
            .repo()
            .map_or_else(|| id.owner().to_owned(), ToString::to_string),
        reference: decision.item.reference(),
        title: decision.item.title.clone(),
        waiting_on: decision.waiting_on,
        waiting_label: if decision.unattributed {
            "unattributed".to_owned()
        } else {
            decision.waiting_on.label().to_owned()
        },
        unattributed: decision.unattributed,
        dimmed: decision.waiting_on == WaitingOn::SomeoneElse,
        incomplete: decision.coverage.iter().any(|c| !c.state.is_complete()),
        age: format::age(state.now, decision.item.updated_at),
        job: live::most_telling(state.job_checks, id).map(|c| jobs::marker(&c.state)),
        landing: landing::row(decision),
    }
}

/// Never says "Nothing needs you" unless every source was read completely.
fn empty_state(state: &State<'_>) -> EmptyState {
    if let Err(error) = state.config {
        return EmptyState {
            title: "den has no usable config".to_owned(),
            detail: error.to_string(),
        };
    }
    // A failed refresh comes first: its details say how to recover, and the
    // header already names the Scope.
    if let Some(failure) = state.failure {
        return EmptyState {
            title: header::failure_text(failure),
            detail: failure.to_string(),
        };
    }
    if let Some(narrowed) = scope::empty_in_scope(state) {
        return narrowed;
    }
    match state.observation {
        None if state.surveying => EmptyState {
            title: format!("Surveying{ELLIPSIS}"),
            detail: "Reading open Pull Requests through gh.".to_owned(),
        },
        None => EmptyState {
            title: "No survey yet.".to_owned(),
            detail: "den has not looked at any Owners.".to_owned(),
        },
        Some(observation) if cache::survey_complete(observation) => EmptyState {
            title: "Nothing needs you.".to_owned(),
            detail: format!(
                "Every source was read completely, {}.",
                format::ago(state.now, observation.observed_at)
            ),
        },
        Some(_) => EmptyState {
            title: "No Work Items found, but the picture is incomplete.".to_owned(),
            detail: header::summarise(state),
        },
    }
}
