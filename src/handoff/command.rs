//! The command that starts a Job's Claude Code session (pure). The launch sheet
//! shows exactly what the Launcher runs because both come from here.
//!
//! In kitty: a new kitty OS window (a new kitty process, so no remote control is
//! needed) with its working directory set to the worktree, running
//! `claude -n <session> <prompt>`. Outside kitty: `cd <worktree> && claude -n
//! <session> <prompt>`, for the clipboard. Reopening a Live Job (#14) runs
//! `claude --resume <session>` instead. No permission-bypass flags, ever.
//!
//! A coordination Job (#39) starts in the approval-required permission mode:
//! `claude --permission-mode manual -n <session> <prompt>`, and reopening it
//! passes the mode again. [`PermissionMode`] has no other value, so no bypass
//! mode can be expressed. A coding Job gets no mode flag: milestone 1's
//! command, and the user's own Claude Code settings decide.
//!
//! A Job whose worktree den doesn't trust ([`super::trust`]) starts with
//! `--setting-sources user` ([`SettingSources::UserOnly`]), so the worktree's
//! `.claude/settings.json` and `.claude/settings.local.json` (and the hooks
//! in them) are not loaded. Reopening it passes the flag again.

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
/// Claude Code's flag for the session's permission mode (`claude --help`,
/// Claude Code 2.1.325: choices acceptEdits, auto, bypassPermissions, manual,
/// dontAsk, plan).
const CLAUDE_PERMISSION_MODE_FLAG: &str = "--permission-mode";
/// Claude Code's Manual mode, which asks before most actions that edit files,
/// run shell commands or reach the network. `manual` is the CLI's alias for the
/// config value `default` (code.claude.com/docs/en/permission-modes).
const CLAUDE_MANUAL_MODE: &str = "manual";
/// Claude Code's flag for which settings files to load (`claude --help`:
/// "Comma-separated list of setting sources to load (user, project, local)").
const CLAUDE_SETTING_SOURCES_FLAG: &str = "--setting-sources";
/// Only the user's own settings (`~/.claude/settings.json`).
const CLAUDE_USER_SOURCE: &str = "user";

/// Which of Claude Code's setting sources a Job's session loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SettingSources {
    /// Claude Code's default (no flag): user, project and local.
    #[default]
    All,
    /// `--setting-sources user`: not the worktree's project or local settings.
    UserOnly,
}

impl SettingSources {
    pub fn is_all(&self) -> bool {
        *self == Self::All
    }

    /// The flag and its value, as passed to `claude`; none for [`Self::All`].
    pub fn args(self) -> Vec<String> {
        match self {
            Self::All => Vec::new(),
            Self::UserOnly => vec![
                CLAUDE_SETTING_SOURCES_FLAG.to_owned(),
                CLAUDE_USER_SOURCE.to_owned(),
            ],
        }
    }
}

/// A permission mode den passes to Claude Code. Only the approval-required
/// mode exists: den never passes a bypass mode, nor `dontAsk`.
///
/// Job records store it (`JobRecord::permission_mode`) under a name pinned by
/// `#[serde(rename)]`, so a Rust rename never changes the file. **Adding a
/// variant bumps `jobs::JOBS_FORMAT`**: an older den would fail on the
/// unknown name and could not say the file is from a newer den.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    /// Claude asks before acting (Claude Code's Manual mode), for coordination
    /// Jobs, which act on GitHub and beyond rather than on code. The alias
    /// reads records written before the name was pinned.
    #[serde(rename = "approval_required", alias = "ApprovalRequired")]
    ApprovalRequired,
}

impl PermissionMode {
    /// As the sheet names it.
    pub fn label(self) -> &'static str {
        match self {
            Self::ApprovalRequired => "approval-required",
        }
    }

    /// The flag and its value, as passed to `claude`.
    pub fn args(self) -> [String; 2] {
        let value = match self {
            Self::ApprovalRequired => CLAUDE_MANUAL_MODE,
        };
        [CLAUDE_PERMISSION_MODE_FLAG.to_owned(), value.to_owned()]
    }
}

/// What a Job's window runs, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowCommand {
    pub directory: PathBuf,
    pub session: String,
    /// The prompt for a new session; unused when resuming.
    pub prompt: String,
    pub start: SessionStart,
    /// `None`: no mode flag, as in milestone 1 (the user's settings decide).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// [`SettingSources::All`]: no flag, as in milestone 1.
    #[serde(default, skip_serializing_if = "SettingSources::is_all")]
    pub setting_sources: SettingSources,
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

/// `claude -n <session> <prompt>` (or `claude --resume <session>`), after
/// `--permission-mode <mode>` and `--setting-sources user` when the window
/// has them, as arguments after the program.
pub fn claude_args(window: &WindowCommand) -> Vec<String> {
    let mode = window
        .permission_mode
        .into_iter()
        .flat_map(PermissionMode::args)
        .chain(window.setting_sources.args());
    let session = match window.start {
        SessionStart::New => vec![
            CLAUDE_NAME_FLAG.to_owned(),
            window.session.clone(),
            window.prompt.clone(),
        ],
        SessionStart::Resume => vec![CLAUDE_RESUME_FLAG.to_owned(), window.session.clone()],
    };
    mode.chain(session).collect()
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
