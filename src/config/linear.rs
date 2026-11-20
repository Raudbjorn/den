//! An Owner's Linear Workflow block (spec 0002, "Config").
//!
//! ```toml
//! owners = ["acme", "me"]
//!
//! [linear.acme]                    # the Owner, as listed in `owners`
//! workspace = "acme"               # the Linear workspace's URL key
//! api_key_env = "DEN_LINEAR_KEY"   # the environment variable holding the API key
//! start_teams = ["TEAM"]           # team keys offering unassigned work (Start)
//! include_backlog = false          # also offer backlog-state issues (default false)
//! active_cycle_only = false        # only unassigned issues in the active cycle (default false)
//! new_work_repos = ["api", "web"]  # Repos offered for Start and Implement
//! held_repos = ["app"]             # Held Repos: never merged with their group
//! landing_order = ["api"]          # Repos that land first, in order
//! reviewers = ["alice", "acme/backend"]  # GitHub logins or org/team
//! slack_channel = "#team-dev"      # optional, opaque: only passed into prompts
//! in_progress_state = "In Progress"
//! release_state = "Ready for Release"
//! exact_head_approval = true       # default true
//! comment_reviews_need_rereview = true  # default true
//! ```
//!
//! Every Repo named must be the Owner's own: `api` or `acme/api`, never
//! another Owner's. Team and state names are checked against Linear on a
//! survey (#33, the team-states source), not here. No organization's values are defaults: only the
//! two policy switches have defaults, and those are the config's (on), not
//! the policy type's (whose empty default changes nothing).

use super::{Location, location_of};
use crate::domain::{
    LinearSourceKind, LinearWorkflowPolicy, RepoId, ReviewPolicy, StartPolicy, TeamRef,
    ids::{is_valid_login, is_valid_repo_name},
};
use serde::Deserialize;
use std::{collections::BTreeMap, ops::Range};
use toml::Spanned;

/// den's bound on a Linear workspace URL key, in characters.
const MAX_WORKSPACE_CHARS: usize = 64;
/// den's bound on an environment variable name, in characters.
const MAX_ENV_NAME_CHARS: usize = 128;
/// den's bound on a Linear team key, in characters.
const MAX_TEAM_KEY_CHARS: usize = 32;
/// den's bound on a team slug, in characters.
const MAX_TEAM_SLUG_CHARS: usize = 100;
/// den's bound on a state name or Slack channel, in characters.
const MAX_SETTING_CHARS: usize = 100;

/// One Owner's Linear Workflow, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearWorkflow {
    /// The Owner, spelled as in `owners`.
    pub owner: String,
    /// The Linear workspace's URL key, lowercased.
    pub workspace: String,
    /// The environment variable holding the API key. The key itself is read
    /// only by the Linear source, never stored in the config.
    pub api_key_env: String,
    /// Where `api_key_env` is set, for the missing-key error.
    pub api_key_env_at: Location,
    /// Team keys whose unassigned issues den offers to Start (#33).
    pub start_teams: Vec<String>,
    pub include_backlog: bool,
    pub active_cycle_only: bool,
    /// Repos offered for new work (Start and Implement, #38).
    pub new_work_repos: Vec<RepoId>,
    /// Held Repos (#36).
    pub held_repos: Vec<RepoId>,
    /// Repos that land first, in order (#40).
    pub landing_order: Vec<RepoId>,
    /// Configured reviewers (#36, #39).
    pub reviewers: Vec<Reviewer>,
    /// Opaque; only ever passed into Request review prompts (#39).
    pub slack_channel: Option<String>,
    pub in_progress_state: Option<String>,
    pub release_state: Option<String>,
    /// Approvals count only at the PR's current head (#36). Default on.
    pub exact_head_approval: bool,
    /// A COMMENTED review needs a fresh look after the head moves (#36). Default on.
    pub comment_reviews_need_rereview: bool,
}

impl LinearWorkflow {
    /// The decision engine's policy for this workflow's Owner: which
    /// unassigned Linear Issues it offers to Start (#33), its review rules
    /// (the two switches as configured, and the configured reviewers) and its
    /// Held Repos (#36). The landing order arrives with #40.
    pub fn policy(&self) -> LinearWorkflowPolicy {
        LinearWorkflowPolicy {
            start: StartPolicy {
                teams: self.start_teams.clone(),
                include_backlog: self.include_backlog,
                active_cycle_only: self.active_cycle_only,
            },
            review: Some(ReviewPolicy {
                exact_head_approval: self.exact_head_approval,
                comment_reviews_need_rereview: self.comment_reviews_need_rereview,
                reviewers: self.reviewers.clone(),
            }),
            held_repos: self.held_repos.clone(),
        }
    }

    /// The Linear sources this workflow reads, in survey order: always the
    /// viewer and its assigned issues; the team states when the workflow names
    /// a start team or a state to check against Linear; the start teams'
    /// issues when it names a start team. Every Linear source den records
    /// for the workflow comes from here, so one it no longer reads is dropped
    /// from the cache (`CachedObservation::restricted_to_linear`).
    pub fn linear_sources(&self) -> Vec<LinearSourceKind> {
        let mut sources = vec![LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues];
        if !self.start_teams.is_empty() || self.state_names().next().is_some() {
            sources.push(LinearSourceKind::TeamStates);
        }
        if !self.start_teams.is_empty() {
            sources.push(LinearSourceKind::StartTeamIssues);
        }
        sources
    }

    /// The state names the config gives (setting, name), to check against
    /// Linear's team states.
    pub fn state_names(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            ("in_progress_state", self.in_progress_state.as_deref()),
            ("release_state", self.release_state.as_deref()),
        ]
        .into_iter()
        .filter_map(|(setting, name)| name.map(|n| (setting, n)))
    }
}

/// A configured reviewer is the domain's: a GitHub login or a team, parsed
/// from `"login"` or `"org/team"` (`reviewer`).
pub use crate::domain::facts::Reviewer;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawLinearWorkflow {
    workspace: Spanned<String>,
    api_key_env: Spanned<String>,
    #[serde(default)]
    start_teams: Vec<Spanned<String>>,
    #[serde(default)]
    include_backlog: bool,
    #[serde(default)]
    active_cycle_only: bool,
    #[serde(default)]
    new_work_repos: Vec<Spanned<String>>,
    #[serde(default)]
    held_repos: Vec<Spanned<String>>,
    #[serde(default)]
    landing_order: Vec<Spanned<String>>,
    #[serde(default)]
    reviewers: Vec<Spanned<String>>,
    slack_channel: Option<Spanned<String>>,
    in_progress_state: Option<Spanned<String>>,
    release_state: Option<Spanned<String>>,
    exact_head_approval: Option<bool>,
    comment_reviews_need_rereview: Option<bool>,
}

type Invalid = (Range<usize>, String);

/// Validates every `[linear.<owner>]` block against the watched `owners`, in
/// `owners` order.
pub(super) fn workflows(
    raw: BTreeMap<Spanned<String>, Spanned<RawLinearWorkflow>>,
    owners: &[String],
    text: &str,
) -> Result<Vec<LinearWorkflow>, Invalid> {
    let mut found: Vec<(usize, LinearWorkflow)> = Vec::new();
    for (key, block) in raw {
        let key_span = key.span();
        let key = key.into_inner();
        let Some((position, owner)) = owners
            .iter()
            .enumerate()
            .find(|(_, o)| o.eq_ignore_ascii_case(&key))
        else {
            return Err((
                key_span,
                format!(
                    "[linear.{key}]: {key:?} is not in `owners`; a Linear Workflow belongs to a watched Owner"
                ),
            ));
        };
        if found.iter().any(|(p, _)| *p == position) {
            return Err((
                key_span,
                format!("{owner} has two Linear Workflows; give it one"),
            ));
        }
        let block_span = block.span();
        let workflow = workflow(owner, block.into_inner(), text)?;
        if let Some((_, other)) = found
            .iter()
            .find(|(_, w)| w.workspace == workflow.workspace)
        {
            return Err((
                block_span,
                format!(
                    "the Linear workspace {:?} is bound to both {} and {owner}; a workspace belongs to one Owner's Linear Workflow",
                    workflow.workspace, other.owner
                ),
            ));
        }
        found.push((position, workflow));
    }
    found.sort_by_key(|(position, _)| *position);
    Ok(found.into_iter().map(|(_, w)| w).collect())
}

fn workflow(owner: &str, raw: RawLinearWorkflow, text: &str) -> Result<LinearWorkflow, Invalid> {
    let workspace = workspace(raw.workspace)?;
    let api_key_env_at = location_of(text, raw.api_key_env.span().start);
    let api_key_env = env_name(raw.api_key_env)?;
    let start_teams = unique(raw.start_teams, "start_teams", team_key)?;
    let repos = |list, field| unique(list, field, |entry| repo(owner, field, entry));
    Ok(LinearWorkflow {
        owner: owner.to_owned(),
        workspace,
        api_key_env,
        api_key_env_at,
        start_teams,
        include_backlog: raw.include_backlog,
        active_cycle_only: raw.active_cycle_only,
        new_work_repos: repos(raw.new_work_repos, "new_work_repos")?,
        held_repos: repos(raw.held_repos, "held_repos")?,
        landing_order: repos(raw.landing_order, "landing_order")?,
        reviewers: unique(raw.reviewers, "reviewers", reviewer)?,
        slack_channel: raw
            .slack_channel
            .map(|v| setting(v, "slack_channel"))
            .transpose()?,
        in_progress_state: raw
            .in_progress_state
            .map(|v| setting(v, "in_progress_state"))
            .transpose()?,
        release_state: raw
            .release_state
            .map(|v| setting(v, "release_state"))
            .transpose()?,
        exact_head_approval: raw.exact_head_approval.unwrap_or(true),
        comment_reviews_need_rereview: raw.comment_reviews_need_rereview.unwrap_or(true),
    })
}

/// Validates each entry and refuses duplicates. Entries are compared once
/// validated, case-insensitively, so two spellings of one value (`api` and
/// `acme/api`) are a duplicate too.
fn unique<T: Folded>(
    entries: Vec<Spanned<String>>,
    field: &str,
    check: impl Fn(Spanned<String>) -> Result<T, Invalid>,
) -> Result<Vec<T>, Invalid> {
    let mut seen: Vec<String> = Vec::with_capacity(entries.len());
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        let span = entry.span();
        let written = entry.get_ref().clone();
        let value = check(entry)?;
        let folded = value.folded();
        if seen.contains(&folded) {
            return Err((span, format!("{written:?} is listed twice in `{field}`")));
        }
        seen.push(folded);
        values.push(value);
    }
    Ok(values)
}

/// A validated list entry's identity for the duplicate check: GitHub and
/// Linear compare these names case-insensitively.
trait Folded {
    fn folded(&self) -> String;
}

impl Folded for String {
    fn folded(&self) -> String {
        self.to_ascii_lowercase()
    }
}

impl Folded for RepoId {
    fn folded(&self) -> String {
        self.to_string().to_ascii_lowercase()
    }
}

impl Folded for Reviewer {
    fn folded(&self) -> String {
        match self {
            Self::User(login) => login.to_ascii_lowercase(),
            Self::Team(team) => format!("{}/{}", team.org, team.slug).to_ascii_lowercase(),
        }
    }
}

fn workspace(value: Spanned<String>) -> Result<String, Invalid> {
    let span = value.span();
    let value = value.into_inner();
    let ok = (1..=MAX_WORKSPACE_CHARS).contains(&value.chars().count())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        return Ok(value.to_ascii_lowercase());
    }
    Err((
        span,
        format!(
            "`workspace` {value:?} is not a Linear workspace URL key (letters, digits and hyphens, at most {MAX_WORKSPACE_CHARS} characters; the part after linear.app/)"
        ),
    ))
}

fn env_name(value: Spanned<String>) -> Result<String, Invalid> {
    let span = value.span();
    let value = value.into_inner();
    let mut chars = value.chars();
    let ok = value.chars().count() <= MAX_ENV_NAME_CHARS
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        return Ok(value);
    }
    Err((
        span,
        format!(
            "`api_key_env` {value:?} is not an environment variable name (letters, digits and _, not starting with a digit); name the variable that holds the key, never the key itself"
        ),
    ))
}

fn team_key(value: Spanned<String>) -> Result<String, Invalid> {
    let span = value.span();
    let value = value.into_inner();
    let ok = (1..=MAX_TEAM_KEY_CHARS).contains(&value.chars().count())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        return Ok(value);
    }
    Err((
        span,
        format!(
            "{value:?} in `start_teams` is not a Linear team key (letters, digits and _, as in the TEAM of TEAM-123)"
        ),
    ))
}

/// `name` or `owner/name`, which must be one of `owner`'s Repos.
fn repo(owner: &str, field: &str, value: Spanned<String>) -> Result<RepoId, Invalid> {
    let span = value.span();
    let value = value.into_inner();
    let (named_owner, name) = match value.split_once('/') {
        Some((o, n)) => (Some(o), n),
        None => (None, value.as_str()),
    };
    if let Some(named) = named_owner
        && !named.eq_ignore_ascii_case(owner)
    {
        return Err((
            span,
            format!(
                "{value:?} in `{field}` is not one of {owner}'s Repos: a Linear Workflow names only its own Owner's Repos"
            ),
        ));
    }
    if !is_valid_repo_name(name) {
        return Err((
            span,
            format!("{value:?} in `{field}` is not a Repo name (`name` or `{owner}/name`)"),
        ));
    }
    Ok(RepoId::new(owner, name))
}

/// A GitHub login, or `org/team-slug`.
fn reviewer(value: Spanned<String>) -> Result<Reviewer, Invalid> {
    let span = value.span();
    let value = value.into_inner();
    let invalid = || {
        (
            span.clone(),
            format!("{value:?} in `reviewers` is not a GitHub login or an org/team"),
        )
    };
    match value.split_once('/') {
        None if is_valid_login(&value) => Ok(Reviewer::User(value.clone())),
        None => Err(invalid()),
        Some((org, slug)) => {
            let slug_ok = (1..=MAX_TEAM_SLUG_CHARS).contains(&slug.chars().count())
                && slug
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
            if is_valid_login(org) && slug_ok {
                Ok(Reviewer::Team(TeamRef {
                    org: org.to_owned(),
                    slug: slug.to_owned(),
                }))
            } else {
                Err(invalid())
            }
        }
    }
}

/// A free-text setting: not blank, no control characters, bounded.
fn setting(value: Spanned<String>, field: &str) -> Result<String, Invalid> {
    let span = value.span();
    let value = value.into_inner();
    let ok = !value.trim().is_empty()
        && value.chars().count() <= MAX_SETTING_CHARS
        && !value.chars().any(char::is_control);
    if ok {
        return Ok(value);
    }
    Err((
        span,
        format!(
            "`{field}` must be non-empty text without control characters, at most {MAX_SETTING_CHARS} characters"
        ),
    ))
}
