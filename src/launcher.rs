//! The real Launcher: a new kitty OS window, or the clipboard outside kitty.
//!
//! In kitty (`KITTY_WINDOW_ID` is set and `kitty` is on PATH) den starts a new
//! kitty process with `--detach`, so no remote control is needed. Outside kitty
//! the command goes to the clipboard: `wl-copy` on Wayland, else `xclip` or
//! `xsel` on X11. Every call has a timeout. What runs is decided by the pure
//! functions here ([`window_mode`], [`clipboard_invocation`], and
//! [`crate::handoff::command::kitty_invocation`]), which the tests check.

use crate::{
    handoff::{
        WindowCommand, WindowMode,
        command::{self, CLAUDE_PROGRAM, KITTY_PROGRAM},
    },
    ports::{LaunchError, Launcher},
    process::{self, Invocation},
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

/// The most opening a kitty window may take, in seconds (kitty returns as soon
/// as it has detached).
pub const KITTY_TIMEOUT_SECS: u64 = 15;
/// The most a clipboard tool may take to take the text, in seconds.
pub const CLIPBOARD_TIMEOUT_SECS: u64 = 5;
const WL_COPY: &str = "wl-copy";
const XCLIP: &str = "xclip";
const XSEL: &str = "xsel";

/// What den knows about where it runs, from its environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEnvironment {
    /// kitty sets `KITTY_WINDOW_ID` in the programs it runs.
    pub in_kitty: bool,
    /// `WAYLAND_DISPLAY` is set.
    pub wayland: bool,
    /// `DISPLAY` is set.
    pub x11: bool,
    /// The folders of `PATH`, in order.
    pub path: Vec<PathBuf>,
}

impl LaunchEnvironment {
    /// From environment variables (`std::env::var_os` in production).
    pub fn from_env(lookup: impl Fn(&str) -> Option<OsString>) -> Self {
        let set = |name: &str| lookup(name).is_some_and(|v| !v.is_empty());
        Self {
            in_kitty: set("KITTY_WINDOW_ID"),
            wayland: set("WAYLAND_DISPLAY"),
            x11: set("DISPLAY"),
            path: lookup("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default(),
        }
    }

    /// Where `program` is on PATH, if it is an executable file there.
    pub fn find(&self, program: &str) -> Option<PathBuf> {
        self.path
            .iter()
            .map(|dir| dir.join(program))
            .find(|candidate| is_executable(candidate))
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    /// Any execute bit.
    const EXECUTE_BITS: u32 = 0o111;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & EXECUTE_BITS != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// A window opens when den runs inside kitty and kitty is on PATH; otherwise
/// the clipboard is used, and the sheet says why.
pub fn window_mode(environment: &LaunchEnvironment, kitty_found: bool) -> WindowMode {
    match (environment.in_kitty, kitty_found) {
        (true, true) => WindowMode::Kitty,
        (true, false) => WindowMode::Clipboard {
            why: "den runs in kitty but `kitty` is not on PATH".to_owned(),
        },
        (false, _) => WindowMode::Clipboard {
            why: "den is not running in kitty".to_owned(),
        },
    }
}

/// The clipboard tool to run (the text goes to its stdin): `wl-copy` on
/// Wayland, else `xclip`, else `xsel` on X11; an error naming what to install
/// when there is none.
pub fn clipboard_invocation(
    environment: &LaunchEnvironment,
    available: impl Fn(&str) -> bool,
) -> Result<Invocation, String> {
    if environment.wayland && available(WL_COPY) {
        return Ok(Invocation::new(WL_COPY).discard_output());
    }
    if environment.x11 && available(XCLIP) {
        return Ok(Invocation::new(XCLIP)
            .args(["-selection", "clipboard"])
            .discard_output());
    }
    if environment.x11 && available(XSEL) {
        return Ok(Invocation::new(XSEL)
            .args(["--clipboard", "--input"])
            .discard_output());
    }
    Err(match (environment.wayland, environment.x11) {
        (true, _) => "no clipboard tool: install wl-clipboard (wl-copy)".to_owned(),
        (false, true) => "no clipboard tool: install xclip or xsel".to_owned(),
        (false, false) => {
            "no graphical session to copy to (neither WAYLAND_DISPLAY nor DISPLAY is set)"
                .to_owned()
        }
    })
}

/// The kitty-or-clipboard adapter.
#[derive(Debug, Clone)]
pub struct KittyLauncher {
    environment: LaunchEnvironment,
}

impl KittyLauncher {
    pub fn new(environment: LaunchEnvironment) -> Self {
        Self { environment }
    }
}

impl Launcher for KittyLauncher {
    fn window_mode(&self) -> WindowMode {
        window_mode(
            &self.environment,
            self.environment.find(KITTY_PROGRAM).is_some(),
        )
    }

    fn claude_available(&self) -> bool {
        self.environment.find(CLAUDE_PROGRAM).is_some()
    }

    fn open_window(&self, window: &WindowCommand) -> Result<(), LaunchError> {
        let invocation = command::kitty_invocation(window);
        let error = |detail: String| LaunchError {
            operation: format!("open a kitty window in {}", window.directory.display()),
            detail,
        };
        let output =
            process::run_invocation(&invocation, &[], Duration::from_secs(KITTY_TIMEOUT_SECS))
                .map_err(|e| error(e.to_string()))?;
        if output.output_left_open {
            tracing::warn!("kitty's detached window kept den's output pipes open");
        }
        if output.status.success() {
            Ok(())
        } else {
            let stderr = output.stderr_text();
            Err(error(if stderr.is_empty() {
                format!("kitty exited with {}", output.status)
            } else {
                stderr
            }))
        }
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<(), LaunchError> {
        let error = |detail: String| LaunchError {
            operation: "copy the command to the clipboard".to_owned(),
            detail,
        };
        let invocation =
            clipboard_invocation(&self.environment, |p| self.environment.find(p).is_some())
                .map_err(error)?;
        let output = process::run_invocation(
            &invocation,
            text.as_bytes(),
            Duration::from_secs(CLIPBOARD_TIMEOUT_SECS),
        )
        .map_err(|e| error(e.to_string()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(error(format!(
                "{} exited with {}",
                invocation.program_name(),
                output.status
            )))
        }
    }
}
