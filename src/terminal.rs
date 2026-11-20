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
            // Raw mode may already be on; undo whatever did happen, and report
            // both failures if undoing it fails too.
            Err(init_error) => match ratatui::try_restore() {
                Ok(()) => Err(init_error),
                Err(restore_error) => Err(io::Error::new(
                    init_error.kind(),
                    format!(
                        "{init_error}; restoring the terminal afterwards also failed: {restore_error}"
                    ),
                )),
            },
        }
    }

    pub fn terminal(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }

    /// Leaves raw mode and the alternate screen. On failure, dropping the
    /// session tries once more.
    pub fn finish(mut self) -> io::Result<()> {
        let restored = ratatui::try_restore();
        self.restored = restored.is_ok();
        restored
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
