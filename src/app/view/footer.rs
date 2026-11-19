//! Key hints.

use super::State;
use crate::app::view_model::{Footer, KeyHint};

pub fn build(state: &State<'_>) -> Footer {
    let mut hints = Vec::new();
    if !state.decisions.is_empty() {
        hints.push(hint("↑↓", "select"));
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
