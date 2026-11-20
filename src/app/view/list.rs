//! The Next Step list and its empty states.

use super::{State, format, header};
use crate::{
    app::view_model::{EmptyState, List, Row},
    decide::Decision,
    domain::WaitingOn,
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
        repo: id.repo().to_string(),
        reference: id.reference(),
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
    if let Some(failure) = state.failure {
        return EmptyState {
            title: failure.headline().to_owned(),
            detail: failure.to_string(),
        };
    }
    match state.observation {
        None if state.surveying => EmptyState {
            title: "Surveying…".to_owned(),
            detail: "Reading open Pull Requests through gh.".to_owned(),
        },
        None => EmptyState {
            title: "No survey yet.".to_owned(),
            detail: "den has not looked at any Owners.".to_owned(),
        },
        Some(observation) if observation.coverage.is_complete() => EmptyState {
            title: "Nothing needs you.".to_owned(),
            detail: "Every source was read completely.".to_owned(),
        },
        Some(observation) => EmptyState {
            title: "No Work Items found, but the picture is incomplete.".to_owned(),
            detail: header::summarise(&observation.coverage),
        },
    }
}
