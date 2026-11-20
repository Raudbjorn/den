//! Raw mode and the alternate screen, restored on every exit path.

use ratatui::DefaultTerminal;
use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// Signals that ask den to stop. Their default action kills the process without
/// running `Drop`, which would leave the terminal in raw mode on the alternate
/// screen, so the session catches them and the event loop restores and exits.
/// (In raw mode Ctrl-C arrives as a key, not as SIGINT.)
const TERMINATION_SIGNALS: [i32; 4] = [SIGTERM, SIGHUP, SIGINT, SIGQUIT];

/// Shell convention: a process stopped by signal N exits with status 128 + N.
pub const SIGNAL_EXIT_STATUS_BASE: i32 = 128;

/// The live terminal. Call [`TerminalSession::finish`] to restore it and learn
/// whether that worked; dropping it restores too (best effort, for early returns).
pub struct TerminalSession {
    terminal: DefaultTerminal,
    restored: bool,
    /// The last termination signal received, or 0 for none.
    signal: Arc<AtomicUsize>,
}

impl TerminalSession {
    /// Enters raw mode and the alternate screen. Also installs ratatui's panic
    /// hook, which restores the terminal before a panic message is printed, and
    /// catches the termination signals (see [`TerminalSession::termination_signal`]).
    pub fn start() -> io::Result<Self> {
        let signal = catch_termination_signals()?;
        match ratatui::try_init() {
            Ok(terminal) => Ok(Self {
                terminal,
                restored: false,
                signal,
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

    /// The termination signal received since the session started, if any. The
    /// event loop must check this and return, so the terminal gets restored. A
    /// second termination signal ends the process at once, unrestored.
    pub fn termination_signal(&self) -> Option<i32> {
        match self.signal.load(Ordering::SeqCst) {
            0 => None,
            signal => i32::try_from(signal).ok(),
        }
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

/// Registers the termination signals for the rest of the process's life (den has
/// one terminal session per process). The first one is recorded; a second one,
/// of any of them, exits immediately in case the loop is stuck.
fn catch_termination_signals() -> io::Result<Arc<AtomicUsize>> {
    let received = Arc::new(AtomicUsize::new(0));
    let requested = Arc::new(AtomicBool::new(false));
    for signal in TERMINATION_SIGNALS {
        // Registered first, so it runs before this signal sets `requested`: only a
        // second signal finds it set.
        signal_hook::flag::register_conditional_shutdown(
            signal,
            SIGNAL_EXIT_STATUS_BASE + signal,
            Arc::clone(&requested),
        )?;
        signal_hook::flag::register(signal, Arc::clone(&requested))?;
        let number = usize::try_from(signal).map_err(io::Error::other)?;
        signal_hook::flag::register_usize(signal, Arc::clone(&received), number)?;
    }
    Ok(received)
}
