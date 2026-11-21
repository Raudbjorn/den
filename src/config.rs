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
//! # [repos."octo/app"]                Next Steps a Repo doesn't track (spec 0003)
//! # skip = ["Fix CI", "Triage"]
//! # screenshots = true                 my PRs here need screenshots (#79)
//! ```
//!
//! A Linear Workflow binds one Owner to one Linear workspace (spec 0002,
//! "Config"); see [`linear`] for the block. The API key is never in the file:
//! the block names the environment variable that holds it.

pub mod linear;

pub use linear::{LinearWorkflow, Reviewer};

use crate::domain::{
    NextStep, Policies, RepoId, Skips,
    facts::feedback::{DEN_MARKER_PREFIX, is_marker_prefix},
    ids::{MAX_LOGIN_CHARS, is_valid_login, is_valid_repo_name},
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
    /// The Next Step kinds each Repo doesn't track (`[repos."OWNER/REPO"]
    /// skip`, spec 0003). Repos outside the watched Owners may be listed.
    pub skips: Skips,
    /// The Repos with `screenshots = true` (#79): the viewer's own PRs there
    /// need a Screenshots section with an image, or a "No visual change" line.
    pub screenshot_repos: Vec<RepoId>,
    /// `legacy_marker_prefixes` (#75, ADR-0010): earlier tooling's marker
    /// prefixes whose `<prefix>:review-notes:` markers set Review Points.
    /// Lowercase letters, digits and `-`, never `den`.
    pub legacy_marker_prefixes: Vec<String>,
}

impl Config {
    /// The Linear Workflow of `owner`, if it has one.
    pub fn linear_workflow(&self, owner: &str) -> Option<&LinearWorkflow> {
        self.linear
            .iter()
            .find(|w| w.owner.eq_ignore_ascii_case(owner))
    }

    /// Each Owner's Linear Workflow policy for the decision engine; the empty
    /// policy for Owners without a Linear Workflow. Each workflow maps its own
    /// ([`LinearWorkflow::policy`]).
    pub fn policies(&self) -> Policies {
        self.linear
            .iter()
            .fold(Policies::default(), |policies, w| {
                policies.with(w.owner.clone(), w.policy())
            })
            .with_skips(self.skips.clone())
            .with_screenshot_repos(self.screenshot_repos.clone())
            .with_legacy_marker_prefixes(self.legacy_marker_prefixes.clone())
    }

    /// The environment variables a Job's command removes (#70, ADR-0007):
    /// every Linear Workflow's API key variable, each once, sorted. A Job
    /// never needs den's Linear key: den makes its Linear writes itself.
    pub fn job_removed_env(&self) -> Vec<String> {
        self.linear
            .iter()
            .map(|workflow| workflow.api_key_env.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
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
    #[serde(default)]
    repos: BTreeMap<Spanned<String>, RawRepo>,
    #[serde(default)]
    legacy_marker_prefixes: Vec<Spanned<String>>,
}

/// One `[repos."OWNER/REPO"]` table (spec 0003).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepo {
    #[serde(default)]
    skip: Vec<Spanned<String>>,
    /// Off unless written (#79).
    #[serde(default)]
    screenshots: bool,
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
    let (skips, screenshot_repos) =
        repo_tables(raw.repos).map_err(|(span, message)| malformed(Some(span), message))?;
    let legacy_marker_prefixes = marker_prefixes(raw.legacy_marker_prefixes)
        .map_err(|(span, message)| malformed(Some(span), message))?;
    Ok(Config {
        owners,
        main_clone_root,
        main_clones,
        linear,
        skips,
        screenshot_repos,
        legacy_marker_prefixes,
    })
}

/// `legacy_marker_prefixes` (#75): each lowercase letters, digits and `-`,
/// not den's own prefix, none twice.
fn marker_prefixes(raw: Vec<Spanned<String>>) -> Result<Vec<String>, (Range<usize>, String)> {
    let mut prefixes: Vec<String> = Vec::with_capacity(raw.len());
    for entry in raw {
        let span = entry.span();
        let prefix = entry.into_inner();
        if !is_marker_prefix(&prefix) {
            return Err((
                span,
                format!(
                    "{prefix:?} is not a marker prefix: use lowercase letters, digits and -, as in <!-- {{prefix}}:review-notes:<review id>:addressed -->"
                ),
            ));
        }
        if prefix == DEN_MARKER_PREFIX {
            return Err((
                span,
                format!("{prefix:?} is den's own marker prefix, not a legacy one: leave it out"),
            ));
        }
        if prefixes.contains(&prefix) {
            return Err((
                span,
                format!("{prefix:?} is listed twice in `legacy_marker_prefixes`"),
            ));
        }
        prefixes.push(prefix);
    }
    Ok(prefixes)
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

/// The `[repos."OWNER/REPO"]` tables: their `skip` lists (spec 0003, "Not
/// tracking a flow"), each value a skippable Next Step spelled as
/// `CONTEXT.md` spells it, and the Repos with `screenshots = true` (#79).
fn repo_tables(
    raw: BTreeMap<Spanned<String>, RawRepo>,
) -> Result<(Skips, Vec<RepoId>), (Range<usize>, String)> {
    let mut skips = Skips::default();
    let mut screenshot_repos = Vec::new();
    for (key, table) in raw {
        let key_span = key.span();
        let name = key.into_inner();
        let repo = match name.split_once('/') {
            Some((owner, repo)) if is_valid_login(owner) && is_valid_repo_name(repo) => {
                RepoId::new(owner, repo)
            }
            _ => {
                return Err((
                    key_span,
                    format!("{name:?} is not OWNER/REPO, such as \"octo/app\""),
                ));
            }
        };
        if skips.lists(&repo) {
            return Err((
                key_span,
                format!("{name} is configured twice (Repo names compare without case)"),
            ));
        }
        let mut steps = Vec::with_capacity(table.skip.len());
        for entry in table.skip {
            let span = entry.span();
            let label = entry.into_inner();
            let Some(step) = NextStep::from_label(&label) else {
                return Err((
                    span,
                    format!(
                        "{label:?} is not a Next Step: use one of {}",
                        skippable_names()
                    ),
                ));
            };
            if !step.skippable() {
                return Err((
                    span,
                    format!(
                        "{label} belongs to Linear Issues, not to a Repo, so a Repo can't skip it"
                    ),
                ));
            }
            steps.push(step);
        }
        if table.screenshots {
            screenshot_repos.push(repo.clone());
        }
        skips = skips.with(repo, steps);
    }
    Ok((skips, screenshot_repos))
}

/// The skippable Next Steps, quoted, in glossary order.
fn skippable_names() -> String {
    NextStep::ALL
        .into_iter()
        .filter(|step| step.skippable())
        .map(|step| format!("{:?}", step.label()))
        .collect::<Vec<_>>()
        .join(", ")
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
