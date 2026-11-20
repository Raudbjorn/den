//! The detail pane: the selected item's Next Step, Waiting On and Explanation.

use super::{State, coverage};
use crate::app::view_model::{Detail, DetailContent};

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
                reference: decision.item.id.to_string(),
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
            }
        });
    Detail {
        label: LABEL.to_owned(),
        content,
        placeholder: PLACEHOLDER.to_owned(),
    }
}
