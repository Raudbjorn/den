//! Where den reads its config and writes its state (XDG Base Directory spec).
//!
//! - config: `$XDG_CONFIG_HOME/den/config.toml`, else `~/.config/den/config.toml`
//! - prompts: `prompts.toml` beside the config
//! - state (log, cache in `cache/`, Job records in `jobs.json`): `$XDG_STATE_HOME/den/`, else `~/.local/state/den/`
//! - the user's Claude directory, read for the workflow availability check
//!   (#41): `$CLAUDE_CONFIG_DIR`, else `~/.claude`, as Claude Code finds it
//!
//! Relative XDG values are ignored, as the spec requires.

use crate::handoff::prompts::PROMPTS_FILE;
use std::{ffi::OsString, path::PathBuf};

const APP_DIR: &str = "den";
const CONFIG_FILE: &str = "config.toml";
/// Where Main Clones are searched for when the config sets no `main_clone_root`.
const DEFAULT_CLONE_ROOT: &str = "dev";
const LOG_FILE: &str = "den.log";
const DEFAULT_CONFIG_HOME: &str = ".config";
const DEFAULT_STATE_HOME: &str = ".local/state";
/// Claude Code's own directory under HOME, when `CLAUDE_CONFIG_DIR` is unset.
const DEFAULT_CLAUDE_DIR: &str = ".claude";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_file: PathBuf,
    /// One prompt template per Next Step kind, beside the config.
    pub prompts_file: PathBuf,
    pub state_dir: PathBuf,
    /// `~/dev`, the default Main Clone search root; `None` without a HOME.
    pub default_clone_root: Option<PathBuf>,
    /// `$CLAUDE_CONFIG_DIR`, else `~/.claude`; `None` when neither is an
    /// absolute path. Only read, never written.
    pub claude_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathsError {
    #[error(
        "neither {xdg} nor HOME is set to an absolute path, so den cannot locate its {purpose} directory"
    )]
    NoHome {
        xdg: &'static str,
        purpose: &'static str,
    },
}

impl Paths {
    /// Resolves den's paths from environment variables (`lookup` is `std::env::var_os`
    /// in production, a map in tests).
    pub fn from_env(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Self, PathsError> {
        let absolute = |name: &str| lookup(name).map(PathBuf::from).filter(|p| p.is_absolute());
        let home = absolute("HOME");
        let base = |xdg: &'static str, default: &str, purpose: &'static str| {
            absolute(xdg)
                .or_else(|| home.as_ref().map(|h| h.join(default)))
                .ok_or(PathsError::NoHome { xdg, purpose })
        };
        let config_dir = base("XDG_CONFIG_HOME", DEFAULT_CONFIG_HOME, "config")?.join(APP_DIR);
        Ok(Self {
            config_file: config_dir.join(CONFIG_FILE),
            prompts_file: config_dir.join(PROMPTS_FILE),
            state_dir: base("XDG_STATE_HOME", DEFAULT_STATE_HOME, "state")?.join(APP_DIR),
            default_clone_root: home.as_ref().map(|h| h.join(DEFAULT_CLONE_ROOT)),
            claude_dir: absolute("CLAUDE_CONFIG_DIR")
                .or_else(|| home.as_ref().map(|h| h.join(DEFAULT_CLAUDE_DIR))),
        })
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join(LOG_FILE)
    }
}
