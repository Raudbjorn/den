//! Key hints.

use super::State;
use crate::app::{
    Focus,
    view_model::{ENTER_KEY, Footer, KeyHint, Overlay},
};

pub fn build(state: &State<'_>) -> Footer {
    let mut hints = Vec::new();
    if state.scope.focus == Focus::Sidebar {
        hints.push(hint("↑↓", "move"));
        hints.push(hint(&ENTER_KEY.to_string(), "scope to"));
        hints.push(hint("esc", "back"));
    } else if !state.decisions.is_empty() {
        hints.push(hint("↑↓", "select"));
    }
    match state.overlay {
        Some(Overlay::DetailSheet) => hints.push(hint("esc", "close")),
        None if state.scope.focus == Focus::List
            && !state.layout.has_detail_pane()
            && state.selected.is_some() =>
        {
            hints.push(hint(&ENTER_KEY.to_string(), "detail"));
        }
        _ => {}
    }
    if !state.owners.is_empty() {
        hints.push(hint("←→", "scope"));
        hints.push(hint("tab", "repos"));
    }
    if state.config.is_ok() {
        hints.push(hint("r", "refresh"));
    }
    hints.push(hint("q", "quit"));
    Footer { hints }
}

fn hint(key: &str, label: &str) -> KeyHint {
    KeyHint {
        key: key.to_owned(),
        label: label.to_owned(),
    }
}
