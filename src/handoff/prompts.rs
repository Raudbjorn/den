//! The prompts file (`$XDG_CONFIG_HOME/den/prompts.toml`): one template per Next
//! Step kind. A missing file, or a kind it doesn't mention, uses the default; a
//! malformed file is an error with the file and line, never ignored.
//!
//! ```toml
//! address_review = "/gh-resolve {url}"
//! fix_ci = "/diagnosing-bugs CI fails on {url} (branch {branch}). Find out why and fix it."
//! ```
//!
//! Placeholders: `{repo}` (`owner/name`), `{number}`, `{url}`, `{title}`,
//! `{branch}`; `{{` and `}}` are literal braces. Prompts reference the Work Item
//! by URL and identifiers; the defaults never use `{title}`, and nothing can
//! insert Issue or PR body text, so text written by others can't steer a Job.

use crate::config::{Location, location_of};
use crate::domain::NextStep;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};
use toml::Spanned;

pub const PROMPTS_FILE: &str = "prompts.toml";
/// The longest prompt template den accepts, in characters.
pub const MAX_TEMPLATE_CHARS: usize = 4_000;

const PLACEHOLDERS: [&str; 5] = ["repo", "number", "url", "title", "branch"];

/// The file's key for a Next Step kind: "Address review" → `address_review`.
pub fn step_key(step: NextStep) -> String {
    step.label().to_ascii_lowercase().replace(' ', "_")
}

/// The default template for each kind (spec, "Launch handoff").
pub fn default_template(step: NextStep) -> &'static str {
    match step {
        NextStep::AddressReview => "/gh-resolve {url}",
        NextStep::Review => "/code-review {url}",
        NextStep::FixCi => {
            "/diagnosing-bugs CI is failing for {url} (branch {branch}). Find out why and fix it."
        }
        NextStep::Implement => "/implement {url}",
        NextStep::Triage => "/triage {url}",
        NextStep::FinishDraft => {
            "Pull Request {repo}#{number} ({url}) is a draft. You are on its head branch {branch}. Finish the work so it is ready for review."
        }
        NextStep::ResolveConflicts => {
            "Pull Request {repo}#{number} ({url}) conflicts with its base branch. You are on its head branch {branch}. Resolve the conflicts."
        }
        NextStep::Merge => {
            "Pull Request {repo}#{number} ({url}) is approved and its checks pass. You are on its head branch {branch}. Check that it is ready, then merge it."
        }
        NextStep::OpenPr => {
            "Branch {branch} in {repo} has unmerged commits and no Pull Request. Review its commits and open a Pull Request for it."
        }
        NextStep::PruneOrRevive => {
            "Branch {branch} in {repo} has no Pull Request and no commits for over 30 days. Work out whether to revive or prune it, and ask me before deleting anything."
        }
    }
}

/// The values a template's placeholders take for one Work Item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFields {
    pub repo: String,
    pub number: Option<u64>,
    pub url: String,
    pub title: String,
    pub branch: String,
}

/// A template per Next Step kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompts {
    templates: BTreeMap<NextStep, String>,
}

impl Default for Prompts {
    fn default() -> Self {
        Self {
            templates: NextStep::ALL
                .iter()
                .map(|&step| (step, default_template(step).to_owned()))
                .collect(),
        }
    }
}

impl Prompts {
    pub fn template(&self, step: NextStep) -> &str {
        self.templates
            .get(&step)
            .map_or_else(|| default_template(step), String::as_str)
    }

    /// The prompt for `step` with `fields` filled in.
    pub fn render(&self, step: NextStep, fields: &PromptFields) -> String {
        // Templates are validated when loaded, so every placeholder is known.
        fill(self.template(step), fields)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromptsError {
    #[error("cannot read {}: {error}", path.display())]
    Unreadable { path: PathBuf, error: String },
    #[error("{}{}: {message}", path.display(), location.map(|l| format!(", {l}")).unwrap_or_default())]
    Malformed {
        path: PathBuf,
        location: Option<Location>,
        message: String,
    },
}

/// Reads the prompts file; a missing file means the defaults.
pub fn load(path: &Path) -> Result<Prompts, PromptsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(path, &text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Prompts::default()),
        Err(e) => Err(PromptsError::Unreadable {
            path: path.to_owned(),
            error: e.to_string(),
        }),
    }
}

/// Parses prompts-file text; `path` is used only in errors.
pub fn parse(path: &Path, text: &str) -> Result<Prompts, PromptsError> {
    let malformed =
        |span: Option<std::ops::Range<usize>>, message: String| PromptsError::Malformed {
            path: path.to_owned(),
            location: span.map(|s| location_of(text, s.start)),
            message,
        };
    #[derive(Deserialize)]
    #[serde(transparent)]
    struct Raw(BTreeMap<String, Spanned<String>>);
    let Raw(raw) = toml::from_str(text).map_err(|e| malformed(e.span(), e.message().to_owned()))?;

    let mut prompts = Prompts::default();
    for (key, value) in raw {
        let span = value.span();
        let Some(step) = NextStep::ALL.into_iter().find(|&s| step_key(s) == key) else {
            let known: Vec<String> = NextStep::ALL.into_iter().map(step_key).collect();
            return Err(malformed(
                Some(span),
                format!(
                    "{key:?} is not a Next Step kind; use one of {}",
                    known.join(", ")
                ),
            ));
        };
        let template = value.into_inner();
        if template.trim().is_empty() {
            return Err(malformed(Some(span), format!("`{key}` is empty")));
        }
        if template.chars().count() > MAX_TEMPLATE_CHARS {
            return Err(malformed(
                Some(span),
                format!("`{key}` is longer than {MAX_TEMPLATE_CHARS} characters"),
            ));
        }
        if let Err(problem) = check(&template) {
            return Err(malformed(Some(span), format!("`{key}`: {problem}")));
        }
        prompts.templates.insert(step, template);
    }
    Ok(prompts)
}

/// Checks every `{…}` in `template` is a known placeholder.
fn check(template: &str) -> Result<(), String> {
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
            }
            '{' => {
                let name: String = chars.by_ref().take_while(|&c| c != '}').collect();
                if !PLACEHOLDERS.contains(&name.as_str()) {
                    return Err(format!(
                        "unknown placeholder {{{name}}}; use {}",
                        PLACEHOLDERS.map(|p| format!("{{{p}}}")).join(", ")
                    ));
                }
            }
            '}' => return Err("a lone `}`: write `}}` for a literal brace".to_owned()),
            _ => {}
        }
    }
    Ok(())
}

fn fill(template: &str, fields: &PromptFields) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let name: String = chars.by_ref().take_while(|&c| c != '}').collect();
                match name.as_str() {
                    "repo" => out.push_str(&fields.repo),
                    "number" => {
                        out.push_str(&fields.number.map(|n| n.to_string()).unwrap_or_default())
                    }
                    "url" => out.push_str(&fields.url),
                    "title" => out.push_str(&fields.title),
                    "branch" => out.push_str(&fields.branch),
                    _ => {}
                }
            }
            other => out.push(other),
        }
    }
    out
}
