//! Where den reads its config and writes its state (XDG Base Directory spec).
//!
//! - config: `$XDG_CONFIG_HOME/den/config.toml`, else `~/.config/den/config.toml`
//! - state (log, later cache and Job records): `$XDG_STATE_HOME/den/`, else `~/.local/state/den/`
//!
//! Relative XDG values are ignored, as the spec requires.

use std::{ffi::OsString, path::PathBuf};

const APP_DIR: &str = "den";
const CONFIG_FILE: &str = "config.toml";
const LOG_FILE: &str = "den.log";
const DEFAULT_CONFIG_HOME: &str = ".config";
const DEFAULT_STATE_HOME: &str = ".local/state";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_file: PathBuf,
    pub state_dir: PathBuf,
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
        Ok(Self {
            config_file: base("XDG_CONFIG_HOME", DEFAULT_CONFIG_HOME, "config")?
                .join(APP_DIR)
                .join(CONFIG_FILE),
            state_dir: base("XDG_STATE_HOME", DEFAULT_STATE_HOME, "state")?.join(APP_DIR),
        })
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join(LOG_FILE)
    }
}
