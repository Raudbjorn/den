//! Key hints.

use super::State;
use crate::app::{
    Focus,
    view_model::{Footer, KeyHint},
};

pub fn build(state: &State<'_>) -> Footer {
    let mut hints = Vec::new();
    if state.scope.focus == Focus::Sidebar {
        hints.push(hint("↑↓", "move"));
        hints.push(hint("↵", "scope to"));
        hints.push(hint("esc", "back"));
    } else if !state.decisions.is_empty() {
        hints.push(hint("↑↓", "select"));
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
