//! Raw mode and the alternate screen, restored on every exit path.

use ratatui::DefaultTerminal;
use std::io;

/// The live terminal. Call [`TerminalSession::finish`] to restore it and learn
/// whether that worked; dropping it restores too (best effort, for early returns).
pub struct TerminalSession {
    terminal: DefaultTerminal,
    restored: bool,
}

impl TerminalSession {
    /// Enters raw mode and the alternate screen. Also installs ratatui's panic
    /// hook, which restores the terminal before a panic message is printed.
    pub fn start() -> io::Result<Self> {
        match ratatui::try_init() {
            Ok(terminal) => Ok(Self {
                terminal,
                restored: false,
            }),
            Err(init_error) => {
                // Raw mode may already be on; undo whatever did happen.
                let _ = ratatui::try_restore();
                Err(init_error)
            }
        }
    }

    pub fn terminal(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }

    /// Leaves raw mode and the alternate screen.
    pub fn finish(mut self) -> io::Result<()> {
        self.restored = true;
        ratatui::try_restore()
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if !self.restored {
            // Nowhere to report a failure from Drop; `finish` is the reporting path.
            let _ = ratatui::try_restore();
        }
    }
}
