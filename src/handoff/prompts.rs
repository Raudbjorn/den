//! The prompts file (`$XDG_CONFIG_HOME/den/prompts.toml`): one template per Next
//! Step kind. A missing file, or a kind it doesn't mention, uses the default; a
//! malformed file is an error with the file and line, never ignored.
//!
//! ```toml
//! address_review = "/gh-resolve {url}"
//! fix_ci = "/diagnosing-bugs CI fails on {url}; you are on {branch}. Find out why and fix it."
//! ```
//!
//! Placeholders: `{repo}` (`owner/name`), `{number}`, `{url}`, `{title}`,
//! `{branch}`; `{{` and `}}` are literal braces. Prompts reference the Work Item
//! by URL and identifiers; the defaults never use `{title}`, and nothing can
//! insert Issue or PR body text, so text written by others can't steer a Job.
//!
//! `request_review` alone also has `{reviewers}` (the logins and `org/team`s
//! chosen on the sheet, comma-separated), which it must use, and
//! `{announcement}` (#39). With a Slack channel in the Owner's Linear Workflow,
//! `{announcement}` is the best-effort announcement step with the
//! separate-outcomes and no-blind-resend rules; without one, it only asks for
//! the request's outcome, so the prompt has no announcement step. den never
//! touches Slack: the channel is an opaque string in the prompt.

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
/// Placeholders only the Request review template has (#39).
const REQUEST_REVIEW_PLACEHOLDERS: [&str; 2] = ["reviewers", "announcement"];

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
            "/diagnosing-bugs CI is failing for {url}. You are on branch {branch}. Find out why and fix it."
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
        // Milestone 2 (spec 0002, "Prompts"). Start launches an Implement
        // Job (#38); Request review (#39) and Land (#40) are coordination
        // Jobs; Land's full prompt arrives with #40; Mark for release is a
        // Linear write den makes itself, never a Job (#38).
        NextStep::Start => "/implement {url}",
        NextStep::RequestReview => {
            "Pull Request {repo}#{number} ({url}) needs review. Request reviews on GitHub from exactly these reviewers: {reviewers}. Change nothing else on the Pull Request.{announcement}"
        }
        NextStep::Land => "Landing Group {url} is ready to land. Ask me before merging anything.",
        NextStep::MarkForRelease => {
            "Linear Issue {url} can move to its release state. den makes this change itself; do nothing remote."
        }
    }
}

/// What `{announcement}` is when the Owner's Linear Workflow names a Slack
/// channel: the announcement step, then the separate-outcomes and
/// no-blind-resend rules (spec 0002, stories 65 and 66).
fn announcement_step(channel: &str) -> String {
    format!(
        " Then post one short announcement in the Slack channel {channel} naming the Pull Request by its URL and the reviewers you requested. \
The announcement is best-effort: if it fails, the review request still stands. \
Report the two as separate outcomes: the review request (done or failed, with the error), and the announcement (sent, failed, or unknown when you can't tell whether it went out). \
If you aren't sure an announcement went out, don't send it again: report it as unknown."
    )
}

/// What `{announcement}` is without a Slack channel: no announcement step,
/// only the request's outcome.
const NO_ANNOUNCEMENT: &str =
    " Then report whether the review request was made, with the error if it failed.";

/// The values a template's placeholders take for one Work Item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptFields {
    pub repo: String,
    pub number: Option<u64>,
    pub url: String,
    pub title: String,
    pub branch: String,
    /// Request review only: the chosen reviewers, as logins and `org/team`s.
    pub reviewers: Vec<String>,
    /// Request review only: the Owner's Slack channel, opaque.
    pub slack_channel: Option<String>,
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
        if let Err(problem) = check(&template, step) {
            return Err(malformed(Some(span), format!("`{key}`: {problem}")));
        }
        prompts.templates.insert(step, template);
    }
    Ok(prompts)
}

/// Checks every `{…}` in `step`'s `template` is a placeholder it has, and
/// that it uses every placeholder `step` needs ([`required_placeholders`]).
fn check(template: &str, step: NextStep) -> Result<(), String> {
    let only_request_review: &[&str] = if step == NextStep::RequestReview {
        &REQUEST_REVIEW_PLACEHOLDERS
    } else {
        &[]
    };
    let usable: Vec<&str> = PLACEHOLDERS
        .iter()
        .chain(only_request_review)
        .copied()
        .collect();
    let mut used: Vec<String> = Vec::new();
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
                if !usable.contains(&name.as_str()) {
                    let kind = if REQUEST_REVIEW_PLACEHOLDERS.contains(&name.as_str()) {
                        "the request_review-only"
                    } else {
                        "unknown"
                    };
                    return Err(format!(
                        "{kind} placeholder {{{name}}}; use {}",
                        spelled(&usable)
                    ));
                }
                used.push(name);
            }
            '}' => return Err("a lone `}`: write `}}` for a literal brace".to_owned()),
            _ => {}
        }
    }
    match required_placeholders(step)
        .iter()
        .find(|needed| !used.iter().any(|u| u == *needed))
    {
        Some(missing) => Err(format!(
            "`{}` must use {{{missing}}}, or the Job never gets what the sheet chose",
            step_key(step)
        )),
        None => Ok(()),
    }
}

/// The placeholders `step`'s template must use: a Request review Job learns
/// whom to ask only from `{reviewers}`, the list chosen on the sheet.
fn required_placeholders(step: NextStep) -> &'static [&'static str] {
    match step {
        NextStep::RequestReview => &["reviewers"],
        _ => &[],
    }
}

/// `{a}, {b}, …`.
fn spelled(placeholders: &[&str]) -> String {
    placeholders
        .iter()
        .map(|p| format!("{{{p}}}"))
        .collect::<Vec<_>>()
        .join(", ")
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
                    "reviewers" => out.push_str(&fields.reviewers.join(", ")),
                    "announcement" => match &fields.slack_channel {
                        Some(channel) => out.push_str(&announcement_step(channel)),
                        None => out.push_str(NO_ANNOUNCEMENT),
                    },
                    _ => {}
                }
            }
            other => out.push(other),
        }
    }
    out
}
