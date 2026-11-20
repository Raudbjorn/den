//! The command that starts a Job's Claude Code session (pure). The launch sheet
//! shows exactly what the Launcher runs because both come from here.
//!
//! In kitty: a new kitty OS window (a new kitty process, so no remote control is
//! needed) with its working directory set to the worktree, running
//! `claude -n <session> <prompt>`. Outside kitty: `cd <worktree> && claude -n
//! <session> <prompt>`, for the clipboard. Reopening a Live Job (#14) runs
//! `claude --resume <session>` instead. No permission-bypass flags, ever.

use crate::process::Invocation;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const KITTY_PROGRAM: &str = "kitty";
pub const CLAUDE_PROGRAM: &str = "claude";
/// Claude Code's flag for the session's name.
const CLAUDE_NAME_FLAG: &str = "-n";
/// Claude Code's flag for resuming a session by its name (or ID), as
/// `claude --help` describes it in Claude Code 2.1.
const CLAUDE_RESUME_FLAG: &str = "--resume";

/// What a Job's window runs, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowCommand {
    pub directory: PathBuf,
    pub session: String,
    /// The prompt for a new session; unused when resuming.
    pub prompt: String,
    pub start: SessionStart,
}

/// Whether the window starts a new Claude Code session or resumes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SessionStart {
    /// `claude -n <session> <prompt>`.
    #[default]
    New,
    /// `claude --resume <session>`: a Live Job's recorded session (#14).
    Resume,
}

/// How the Job's session will be started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowMode {
    /// den runs inside kitty: a new kitty OS window opens.
    Kitty,
    /// Not inside kitty: the command is copied to the clipboard and shown.
    Clipboard { why: String },
}

/// `claude -n <session> <prompt>` (or `claude --resume <session>`), as
/// arguments after the program.
pub fn claude_args(window: &WindowCommand) -> Vec<String> {
    match window.start {
        SessionStart::New => vec![
            CLAUDE_NAME_FLAG.to_owned(),
            window.session.clone(),
            window.prompt.clone(),
        ],
        SessionStart::Resume => vec![CLAUDE_RESUME_FLAG.to_owned(), window.session.clone()],
    }
}

/// The kitty invocation that opens the Job's window: `--detach` so kitty
/// returns at once and the window outlives den; `--hold` so a session that
/// fails to start stays readable (it leaves a shell in the worktree).
pub fn kitty_invocation(window: &WindowCommand) -> Invocation {
    Invocation::new(KITTY_PROGRAM)
        .args(["--detach", "--hold", "--directory"])
        .arg(window.directory.clone())
        .arg(CLAUDE_PROGRAM)
        .args(claude_args(window))
}

/// The exact command, as a shell line: the kitty invocation in kitty, else the
/// `cd … && claude …` line the clipboard gets.
pub fn display(mode: &WindowMode, window: &WindowCommand) -> String {
    match mode {
        WindowMode::Kitty => {
            let invocation = kitty_invocation(window);
            std::iter::once(invocation.program_name())
                .chain(
                    invocation
                        .args
                        .iter()
                        .map(|a| a.to_string_lossy().into_owned()),
                )
                .map(|word| shell_quote(&word))
                .collect::<Vec<_>>()
                .join(" ")
        }
        WindowMode::Clipboard { .. } => clipboard_text(window),
    }
}

/// What the clipboard gets outside kitty: paste it into a terminal.
pub fn clipboard_text(window: &WindowCommand) -> String {
    let claude = std::iter::once(CLAUDE_PROGRAM.to_owned())
        .chain(claude_args(window))
        .map(|word| shell_quote(&word))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "cd {} && {claude}",
        shell_quote(&window.directory.to_string_lossy())
    )
}

/// Quotes `word` for a POSIX shell: bare when it is only safe characters, else
/// in single quotes with any `'` written as `'\''`.
pub fn shell_quote(word: &str) -> String {
    let safe = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '='));
    if safe {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}
