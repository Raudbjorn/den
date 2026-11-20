//! The user config (`config.toml`): watched Owners, the Main Clone settings,
//! and each Owner's optional Linear Workflow. Missing or malformed config is an
//! error with the file and line, never silently replaced by defaults: without
//! Owners there is nothing to survey.
//!
//! ```toml
//! owners = ["Raudbjorn", "massif-network"]
//! # main_clone_root = "/home/me/dev"   where to look for Main Clones (default ~/dev)
//! # [main_clones]                      per-Repo Main Clone overrides
//! # Both take absolute paths only.
//! # "Raudbjorn/den" = "/home/me/dev/den"
//! ```
//!
//! A Linear Workflow binds one Owner to one Linear workspace (spec 0002,
//! "Config"); see [`linear`] for the block. The API key is never in the file:
//! the block names the environment variable that holds it.

pub mod linear;

pub use linear::{LinearWorkflow, Reviewer};

use crate::domain::{
    Policies,
    ids::{MAX_LOGIN_CHARS, is_valid_login},
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fmt, io,
    ops::Range,
    path::{Path, PathBuf},
};
use toml::Spanned;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Watched Owners, in the order written, without duplicates.
    pub owners: Vec<String>,
    /// Where to search for Main Clones. Read by the launch handoff (#11).
    pub main_clone_root: Option<PathBuf>,
    /// Per-Repo Main Clone overrides keyed by `owner/name`. Read by the launch handoff (#11).
    pub main_clones: BTreeMap<String, PathBuf>,
    /// The Linear Workflows, at most one per Owner, in `owners` order. An
    /// Owner without one is GitHub-only.
    pub linear: Vec<LinearWorkflow>,
}

impl Config {
    /// The Linear Workflow of `owner`, if it has one.
    pub fn linear_workflow(&self, owner: &str) -> Option<&LinearWorkflow> {
        self.linear
            .iter()
            .find(|w| w.owner.eq_ignore_ascii_case(owner))
    }

    /// Each Owner's Linear Workflow policy for the decision engine; the empty
    /// policy for Owners without a Linear Workflow. The policy's rules arrive
    /// with the tickets that read them (#36, #40), each mapped from the
    /// workflow here.
    pub fn policies(&self) -> Policies {
        self.linear.iter().fold(Policies::default(), |policies, w| {
            policies.with(w.owner.clone(), w.policy())
        })
    }

    /// Checks that every Linear Workflow's API key variable is set and not
    /// empty in `env`, without keeping the key. The error names the variable
    /// and the line that names it.
    pub fn check_linear_keys(
        &self,
        path: &Path,
        env: impl Fn(&str) -> Option<OsString>,
    ) -> Result<(), ConfigError> {
        for workflow in &self.linear {
            let set = env(&workflow.api_key_env).is_some_and(|value| !value.is_empty());
            if !set {
                return Err(ConfigError::MissingLinearKey {
                    path: path.to_owned(),
                    location: workflow.api_key_env_at,
                    owner: workflow.owner.clone(),
                    variable: workflow.api_key_env.clone(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    owners: Spanned<Vec<Spanned<String>>>,
    main_clone_root: Option<Spanned<PathBuf>>,
    #[serde(default)]
    main_clones: BTreeMap<String, Spanned<PathBuf>>,
    #[serde(default)]
    linear: BTreeMap<Spanned<String>, Spanned<linear::RawLinearWorkflow>>,
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
    /// A Linear Workflow's API key variable is not set (or is empty).
    #[error(
        "{}, {location}: the Linear Workflow of {owner} reads its API key from the environment variable {variable}, which is not set: set {variable} to a Linear API key before starting den",
        path.display()
    )]
    MissingLinearKey {
        path: PathBuf,
        /// Where the config names the variable.
        location: Location,
        owner: String,
        variable: String,
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
    let main_clone_root = raw
        .main_clone_root
        .map(|root| absolute(root, "main_clone_root"))
        .transpose()
        .map_err(|(span, message)| malformed(Some(span), message))?;
    let mut main_clones = BTreeMap::new();
    for (repo, clone) in raw.main_clones {
        let clone = absolute(clone, &format!("the Main Clone of {repo}"))
            .map_err(|(span, message)| malformed(Some(span), message))?;
        main_clones.insert(repo, clone);
    }
    let linear = linear::workflows(raw.linear, &owners, text)
        .map_err(|(span, message)| malformed(Some(span), message))?;
    Ok(Config {
        owners,
        main_clone_root,
        main_clones,
        linear,
    })
}

/// A configured folder, which must be absolute: a relative one would mean
/// something different to den, to git (run inside the Main Clone) and to the
/// launched window.
fn absolute(path: Spanned<PathBuf>, what: &str) -> Result<PathBuf, (Range<usize>, String)> {
    let span = path.span();
    let path = path.into_inner();
    if path.is_absolute() {
        return Ok(path);
    }
    Err((
        span,
        format!(
            "{what} must be an absolute path, not {:?} (den doesn't expand ~ or resolve paths against the folder it runs in)",
            path.display().to_string()
        ),
    ))
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
