//! Key hints.

use super::State;
use crate::{
    app::{
        Focus, OverlayState,
        launch::{Sheet, SheetState},
        view_model::{ENTER_KEY, Footer, KeyHint},
    },
    handoff::TargetChoice,
    ordering::{Scope, same_repo},
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
        if let Some(label) = sidebar_upstream_hint(state) {
            hints.push(hint("u", label));
        }
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
    if sheet.editing || sheet.reviewers.as_ref().is_some_and(|r| r.editing) {
        return vec![
            hint("⏎", "done"),
            hint("←→", "move"),
            hint("⌫", "delete"),
            hint("esc", "done"),
        ];
    }
    let switch = sheet_upstream_hint(sheet).map(|label| hint("u", label));
    match sheet.state {
        SheetState::Ready { .. } => [hint("⏎", "launch"), hint("e", "edit")]
            .into_iter()
            .chain(sheet.reviewers.as_ref().map(|_| hint("r", "reviewers")))
            .chain(switch)
            .chain([hint("esc", "cancel")])
            .collect(),
        SheetState::LiveJob { .. } if sheet.launch_refused.is_some() => {
            [hint("o", "open existing")]
                .into_iter()
                .chain(switch)
                .chain([hint("esc", "cancel")])
                .collect()
        }
        SheetState::LiveJob { .. } => [hint("o", "open existing"), hint("f", "start fresh")]
            .into_iter()
            .chain(switch)
            .chain([hint("esc", "cancel")])
            .collect(),
        SheetState::JobUnknown { .. } => [hint("f", "start fresh")]
            .into_iter()
            .chain(switch)
            .chain([hint("esc", "cancel")])
            .collect(),
        SheetState::ReopenReady { .. } => vec![hint("⏎", "open"), hint("esc", "cancel")],
        SheetState::Inspecting => switch.into_iter().chain([hint("esc", "cancel")]).collect(),
        SheetState::Launching { .. } | SheetState::Reopening { .. } => Vec::new(),
        SheetState::Refused(_) => switch.into_iter().chain([hint("esc", "close")]).collect(),
        SheetState::Finished { .. } | SheetState::Reopened { .. } => vec![hint("esc", "close")],
    }
}

/// What `u` on the sheet would switch this Job's target to, when it can.
fn sheet_upstream_hint(sheet: &Sheet) -> Option<&'static str> {
    let fork = sheet.fork.as_ref()?;
    match sheet.choice {
        TargetChoice::Upstream => Some("the fork"),
        TargetChoice::OwnRepo => fork.upstream.as_ref().map(|_| "upstream"),
    }
}

/// What `u` in the sidebar would do to the fork under the cursor, if it is one.
fn sidebar_upstream_hint(state: &State<'_>) -> Option<&'static str> {
    let Scope::Repo(id) = &state.scope.cursor else {
        return None;
    };
    let repo = state
        .observation?
        .repos
        .iter()
        .find(|r| same_repo(&r.id, id))
        .filter(|r| r.is_fork)?;
    if state.upstreams.contains(&repo.id) {
        Some("the fork")
    } else {
        repo.upstream().map(|_| "upstream")
    }
}
