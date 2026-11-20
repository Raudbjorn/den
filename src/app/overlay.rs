//! Sheets over the screen, and the layout they depend on (#10): on a terminal too
//! narrow for the detail pane, `Enter` opens the detail as a sheet and `Esc` (or
//! `Enter` again) closes it.

use super::{App, Effect, Key, OverlayState};
use crate::layout::{Layout, TerminalSize};

impl App {
    /// Handles `key` if an overlay claims it; `None` lets the normal keys run.
    /// Keys an open sheet does not claim (`↑`/`↓`, `q`) still work under it:
    /// the detail sheet follows the selection.
    pub(super) fn overlay_key(&mut self, key: Key) -> Option<Vec<Effect>> {
        match (&self.overlay, key) {
            (Some(OverlayState::DetailSheet), Key::Esc | Key::Enter) => {
                self.overlay = None;
                Some(Vec::new())
            }
            (None, Key::Enter) if self.detail_needs_sheet() => {
                self.overlay = Some(OverlayState::DetailSheet);
                Some(Vec::new())
            }
            _ => None,
        }
    }

    pub(super) fn resized(&mut self, size: TerminalSize) -> Vec<Effect> {
        self.size = size;
        // Wide enough again: the detail pane shows what the sheet showed.
        if matches!(self.overlay, Some(OverlayState::DetailSheet))
            && Layout::for_size(size).has_detail_pane()
        {
            self.overlay = None;
        }
        Vec::new()
    }

    /// The terminal has no room for the detail pane and something is selected.
    fn detail_needs_sheet(&self) -> bool {
        !Layout::for_size(self.size).has_detail_pane() && self.selected_index().is_some()
    }
}
