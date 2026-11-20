//! Key hints.

use super::State;
use crate::app::{
    Focus, OverlayState,
    launch::{Sheet, SheetState},
    view_model::{ENTER_KEY, Footer, KeyHint},
};

pub fn build(state: &State<'_>) -> Footer {
    if let Some(OverlayState::Launch(sheet)) = state.overlay {
        return Footer {
            hints: launch_hints(sheet),
        };
    }
    let mut hints = Vec::new();
    if state.scope.focus == Focus::Sidebar {
        hints.push(hint("↑↓", "move"));
        hints.push(hint(&ENTER_KEY.to_string(), "scope to"));
        hints.push(hint("esc", "back"));
    } else if !state.decisions.is_empty() {
        hints.push(hint("↑↓", "select"));
    }
    match state.overlay {
        Some(OverlayState::DetailSheet) => hints.push(hint("esc", "close")),
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
    if state.scope.focus == Focus::List && state.selected.is_some() {
        hints.push(hint("c", "launch"));
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

/// The launch sheet's keys, for where it stands.
fn launch_hints(sheet: &Sheet) -> Vec<KeyHint> {
    if sheet.editing {
        return vec![
            hint("⏎", "done"),
            hint("←→", "move"),
            hint("⌫", "delete"),
            hint("esc", "done"),
        ];
    }
    match sheet.state {
        SheetState::Ready { .. } => vec![
            hint("⏎", "launch"),
            hint("e", "edit"),
            hint("esc", "cancel"),
        ],
        SheetState::Inspecting => vec![hint("esc", "cancel")],
        SheetState::Launching { .. } => Vec::new(),
        SheetState::Refused(_) | SheetState::Finished { .. } => vec![hint("esc", "close")],
    }
}
