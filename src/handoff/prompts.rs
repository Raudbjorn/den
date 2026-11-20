//! The prompts file (`$XDG_CONFIG_HOME/den/prompts.toml`): one template per Next
//! Step kind. A missing file, or a kind it doesn't mention, uses the default; a
//! malformed file is an error with the file and line, never ignored. A template
//! that is well formed but lacks a placeholder its kind requires (a
//! `request_review` without `{reviewers}`, valid before #39) makes only its own
//! kind unusable ([`Prompts::unusable`]): den refuses that kind with the file
//! and line, and every other kind launches as before.
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
//!
//! `review_linear` (#41) is the Review Job's template in an Owner with a
//! Linear Workflow, beside `review` for every other Owner. It alone has
//! `{linear_context}` (for a Pull Request in a Landing Group, its Linear
//! Issues' identifiers and URLs; nothing otherwise) and `{verdict_only}`
//! (the instruction to return a verdict without editing, committing,
//! approving, requesting changes or posting), and it must use both.
//!
//! `implement_linear` (#38) is a Linear Issue's Start or Implement template.
//! It alone has `{identifier}`; defaults carry no issue title or description.

use crate::config::{Location, location_of};
use crate::domain::{LinearWorkflowPolicy, NextStep};
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
/// Placeholders only the Review (Linear) template has (#41).
const REVIEW_LINEAR_PLACEHOLDERS: [&str; 2] = ["linear_context", "verdict_only"];
/// The prompts-file key of the Review (Linear) template.
pub const REVIEW_LINEAR_KEY: &str = "review_linear";
/// The prompts-file key of a Linear Issue's Start or Implement Job.
pub const IMPLEMENT_LINEAR_KEY: &str = "implement_linear";
const IMPLEMENT_LINEAR_PLACEHOLDERS: [&str; 1] = ["identifier"];
pub const DEFAULT_IMPLEMENT_LINEAR: &str = "/implement {identifier} {url} in {repo}";

/// The file's key for a Next Step kind: "Address review" → `address_review`.
pub fn step_key(step: NextStep) -> String {
    step.label().to_ascii_lowercase().replace(' ', "_")
}

/// Which template a Job's prompt comes from: its Next Step's, or, for a
/// Review Job in a Linear Workflow, Review (Linear)'s (#41).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PromptKind {
    Step(NextStep),
    /// A Review Job for a Pull Request of an Owner with a Linear Workflow:
    /// verdict only, with the Linear context.
    ReviewLinear,
    /// A Start or Implement Job for a Linear Issue.
    ImplementLinear,
}

impl From<NextStep> for PromptKind {
    fn from(step: NextStep) -> Self {
        Self::Step(step)
    }
}

impl PromptKind {
    /// Every kind, in the prompts file's order.
    pub fn all() -> impl Iterator<Item = Self> {
        NextStep::ALL
            .into_iter()
            .map(Self::Step)
            .chain([Self::ReviewLinear, Self::ImplementLinear])
    }

    /// The template a Job for `step` uses, given its Owner's Linear Workflow
    /// `policy` (the empty one without a workflow).
    pub fn for_job(step: NextStep, policy: &LinearWorkflowPolicy) -> Self {
        match step {
            NextStep::Review if policy.reviews_verdict_only() => Self::ReviewLinear,
            step => Self::Step(step),
        }
    }

    /// The prompts file's key: `address_review`, `review_linear`.
    pub fn key(self) -> String {
        match self {
            Self::Step(step) => step_key(step),
            Self::ReviewLinear => REVIEW_LINEAR_KEY.to_owned(),
            Self::ImplementLinear => IMPLEMENT_LINEAR_KEY.to_owned(),
        }
    }

    /// The placeholders only this kind has.
    fn own_placeholders(self) -> &'static [&'static str] {
        match self {
            Self::Step(NextStep::RequestReview) => &REQUEST_REVIEW_PLACEHOLDERS,
            Self::ReviewLinear => &REVIEW_LINEAR_PLACEHOLDERS,
            Self::ImplementLinear => &IMPLEMENT_LINEAR_PLACEHOLDERS,
            Self::Step(_) => &[],
        }
    }

    /// The placeholders this kind's template must use, each with what the
    /// Job would lose without it: a Request review Job learns whom to ask
    /// only from `{reviewers}`, a verdict-only Review Job its Linear Issue and
    /// its limits only from `{linear_context}` and `{verdict_only}`.
    fn required_placeholders(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Step(NextStep::RequestReview) => {
                &[("reviewers", "or the Job never gets what the sheet chose")]
            }
            Self::ReviewLinear => &[
                (
                    "linear_context",
                    "or the Job never learns which Linear Issue the Pull Request is for",
                ),
                (
                    "verdict_only",
                    "or the Job is never told to return a verdict only",
                ),
            ],
            Self::Step(_) | Self::ImplementLinear => &[],
        }
    }

    /// The kind that owns `placeholder`, when only one kind has it.
    fn owner_of(placeholder: &str) -> Option<Self> {
        Self::all().find(|kind| kind.own_placeholders().contains(&placeholder))
    }
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

/// The default Review (Linear) template (spec 0002, "Prompts").
const REVIEW_LINEAR_TEMPLATE: &str = "/code-review {url}{linear_context}{verdict_only}";

/// What `{verdict_only}` is (spec 0002, story 70).
const VERDICT_ONLY: &str = " Return your verdict to me only. Don't edit files, commit, push, approve, request changes, comment or post anywhere: I decide what gets published.";

/// What `{linear_context}` is for a Pull Request attached to `issues`
/// (identifier, URL): nothing when it is attached to none.
fn linear_context(issues: &[(String, String)]) -> String {
    let named: Vec<String> = issues
        .iter()
        .map(|(identifier, url)| format!("{identifier} ({url})"))
        .collect();
    match named.as_slice() {
        [] => String::new(),
        [one] => format!(
            " It is attached to Linear Issue {one}: review it against what that issue asks for."
        ),
        many => format!(
            " It is attached to the Linear Issues {}: review it against what they ask for.",
            many.join(", ")
        ),
    }
}

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
    /// Review (Linear) only: the Linear Issues the Pull Request is attached
    /// to, as (identifier, URL).
    pub linear_issues: Vec<(String, String)>,
    /// A Linear Issue's identifier (`TEAM-123`); empty for GitHub Work Items.
    pub identifier: String,
}

/// The default template of any kind.
pub fn default_template_of(kind: PromptKind) -> &'static str {
    match kind {
        PromptKind::Step(step) => default_template(step),
        PromptKind::ReviewLinear => REVIEW_LINEAR_TEMPLATE,
        PromptKind::ImplementLinear => DEFAULT_IMPLEMENT_LINEAR,
    }
}

/// A template per kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompts {
    templates: BTreeMap<PromptKind, String>,
    /// Kinds whose template in the file den can't use, and why; den refuses
    /// to launch them rather than run another prompt than the file's.
    unusable: BTreeMap<PromptKind, PromptsError>,
}

impl Default for Prompts {
    fn default() -> Self {
        Self {
            templates: PromptKind::all()
                .map(|kind| (kind, default_template_of(kind).to_owned()))
                .collect(),
            unusable: BTreeMap::new(),
        }
    }
}

impl Prompts {
    /// Why the file's template for `kind` can't be used, if it can't.
    pub fn unusable(&self, kind: impl Into<PromptKind>) -> Option<&PromptsError> {
        self.unusable.get(&kind.into())
    }

    /// Every unusable template's problem, in kind order.
    pub fn problems(&self) -> impl Iterator<Item = &PromptsError> {
        self.unusable.values()
    }

    pub fn template(&self, kind: impl Into<PromptKind>) -> &str {
        let kind = kind.into();
        self.templates
            .get(&kind)
            .map_or_else(|| default_template_of(kind), String::as_str)
    }

    /// `kind`'s template uses `{placeholder}`.
    pub fn uses(&self, kind: impl Into<PromptKind>, placeholder: &str) -> bool {
        let kind = kind.into();
        // Templates are validated when loaded (defaults included), so this
        // check only lists what they use.
        check(self.template(kind), kind).is_ok_and(|used| used.iter().any(|u| u == placeholder))
    }

    /// The prompt of `kind` with `fields` filled in.
    pub fn render(&self, kind: impl Into<PromptKind>, fields: &PromptFields) -> String {
        // Templates are validated when loaded, so every placeholder is known.
        fill(self.template(kind), fields)
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
        let Some(kind) = PromptKind::all().find(|k| k.key() == key) else {
            let known: Vec<String> = PromptKind::all().map(PromptKind::key).collect();
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
        let used = check(&template, kind)
            .map_err(|problem| malformed(Some(span.clone()), format!("`{key}`: {problem}")))?;
        if let Some(missing) = missing_required(kind, &used) {
            prompts
                .unusable
                .insert(kind, malformed(Some(span), missing));
            continue;
        }
        prompts.templates.insert(kind, template);
    }
    Ok(prompts)
}

/// Checks every `{…}` in `kind`'s `template` is a placeholder it has; returns
/// the placeholders it uses.
fn check(template: &str, kind: PromptKind) -> Result<Vec<String>, String> {
    let usable: Vec<&str> = PLACEHOLDERS
        .iter()
        .chain(kind.own_placeholders())
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
                    let what = match PromptKind::owner_of(&name) {
                        Some(owner) => format!("the {}-only", owner.key()),
                        None => "unknown".to_owned(),
                    };
                    return Err(format!(
                        "{what} placeholder {{{name}}}; use {}",
                        spelled(&usable)
                    ));
                }
                used.push(name);
            }
            '}' => return Err("a lone `}`: write `}}` for a literal brace".to_owned()),
            _ => {}
        }
    }
    Ok(used)
}

/// Why a template of `kind` that uses `used` lacks a placeholder `kind`
/// needs ([`PromptKind::required_placeholders`]), if it does.
fn missing_required(kind: PromptKind, used: &[String]) -> Option<String> {
    let (missing, why) = kind
        .required_placeholders()
        .iter()
        .find(|(needed, _)| !used.iter().any(|u| u == needed))?;
    Some(format!("`{}` must use {{{missing}}}, {why}", kind.key()))
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
                    "identifier" => out.push_str(&fields.identifier),
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
                    "linear_context" => out.push_str(&linear_context(&fields.linear_issues)),
                    "verdict_only" => out.push_str(VERDICT_ONLY),
                    _ => {}
                }
            }
            other => out.push(other),
        }
    }
    out
}
