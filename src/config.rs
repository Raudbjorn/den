//! The user config (`config.toml`): watched Owners, and room for the Main Clone
//! settings later tickets use. Missing or malformed config is an error with the
//! file and line, never silently replaced by defaults: without Owners there is
//! nothing to survey.
//!
//! ```toml
//! owners = ["Raudbjorn", "massif-network"]
//! # main_clone_root = "/home/me/dev"   where to look for Main Clones (default ~/dev)
//! # [main_clones]                      per-Repo Main Clone overrides
//! # "Raudbjorn/den" = "/home/me/dev/den"
//! ```

use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, io,
    ops::Range,
    path::{Path, PathBuf},
};
use toml::Spanned;

/// GitHub's longest login, in characters.
const MAX_LOGIN_CHARS: usize = 39;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Watched Owners, in the order written, without duplicates.
    pub owners: Vec<String>,
    /// Where to search for Main Clones. Read by the launch handoff (#11).
    pub main_clone_root: Option<PathBuf>,
    /// Per-Repo Main Clone overrides keyed by `owner/name`. Read by the launch handoff (#11).
    pub main_clones: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    owners: Spanned<Vec<Spanned<String>>>,
    main_clone_root: Option<PathBuf>,
    #[serde(default)]
    main_clones: BTreeMap<String, PathBuf>,
}

/// A 1-based line and column in the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "no config file at {}: create it with a line such as  owners = [\"your-login\"]",
        path.display()
    )]
    Missing { path: PathBuf },
    #[error("cannot read {}: {error}", path.display())]
    Unreadable { path: PathBuf, error: String },
    #[error("{}{}: {message}", path.display(), location.map(|l| format!(", {l}")).unwrap_or_default())]
    Malformed {
        path: PathBuf,
        /// `None` only when the parser could not point at a place in the file.
        location: Option<Location>,
        message: String,
    },
}

/// Reads and validates the config file.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(path, &text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(ConfigError::Missing {
            path: path.to_owned(),
        }),
        Err(e) => Err(ConfigError::Unreadable {
            path: path.to_owned(),
            error: e.to_string(),
        }),
    }
}

/// Parses and validates config text; `path` is used only in errors.
pub fn parse(path: &Path, text: &str) -> Result<Config, ConfigError> {
    let malformed = |span: Option<Range<usize>>, message: String| ConfigError::Malformed {
        path: path.to_owned(),
        location: span.map(|s| location_of(text, s.start)),
        message,
    };
    let raw: RawConfig =
        toml::from_str(text).map_err(|e| malformed(e.span(), e.message().to_owned()))?;

    let owners_span = raw.owners.span();
    let owner_entries = raw.owners.into_inner();
    if owner_entries.is_empty() {
        return Err(malformed(
            Some(owners_span),
            "`owners` is empty: list at least one GitHub user or organization".to_owned(),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut owners = Vec::with_capacity(owner_entries.len());
    for entry in owner_entries {
        let span = entry.span();
        let login = entry.into_inner();
        if !is_valid_login(&login) {
            return Err(malformed(
                Some(span),
                format!(
                    "{login:?} is not a GitHub login (letters, digits and inner hyphens, at most {MAX_LOGIN_CHARS} characters)"
                ),
            ));
        }
        if !seen.insert(login.to_ascii_lowercase()) {
            return Err(malformed(
                Some(span),
                format!("{login:?} is listed twice in `owners`"),
            ));
        }
        owners.push(login);
    }
    Ok(Config {
        owners,
        main_clone_root: raw.main_clone_root,
        main_clones: raw.main_clones,
    })
}

fn is_valid_login(login: &str) -> bool {
    let len = login.chars().count();
    (1..=MAX_LOGIN_CHARS).contains(&len)
        && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !login.starts_with('-')
        && !login.ends_with('-')
}

pub(crate) fn location_of(text: &str, offset: usize) -> Location {
    let before = text.get(..offset).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map_or(before, |(_, tail)| tail)
        .chars()
        .count()
        + 1;
    Location { line, column }
}
