//! What a launch needs from GitHub's facts, per kind of Work Item (pure). PRs
//! are in `plan::facts`; this module has Issues, Branches and Runs (#12):
//!
//! - an Issue: a new `<type>_<slug>` branch (`implement_42-add-login`) from
//!   the target's default branch;
//! - a Run (always a default-branch workflow): a new `fix-ci_<workflow>`
//!   branch from the default branch;
//! - a Branch: the Branch itself, fetched by name from the target.
//!
//! None of these carry a commit from GitHub, so `inspect` asks the remote
//! where the base branch is, and the sheet shows (and the executor re-checks)
//! that commit like a PR head.

use super::{BaseRef, Refusal, RefusalReason, Target, TargetChoice, names};
use crate::{
    decide::Decision,
    domain::{NextStep, Observation, Repo, WorkItemId},
};

/// Used for a Run's slug when its workflow has no usable name.
const FALLBACK_WORKFLOW_SLUG: &str = "workflow";

/// What a launch needs from GitHub's facts alone.
#[derive(Debug, Clone)]
pub(super) struct Facts {
    pub step: NextStep,
    pub target: Target,
    /// The Issue or PR number, for prompts.
    pub number: Option<u64>,
    pub base: BaseRef,
    /// The base commit when GitHub says it (a PR head); otherwise `inspect`
    /// asks the remote.
    pub base_commit: Option<String>,
    /// The Job's branch, before any suffix.
    pub branch: String,
    /// A new `<type>_<slug>` branch: its name is the worktree name, suffix included.
    pub fresh: bool,
}

pub(super) fn target(repo: &Repo) -> Target {
    Target {
        repo: repo.id.clone(),
        url: repo.url.clone(),
        is_fork: repo.is_fork,
    }
}

/// The facts for launching an Issue, a Branch or a Run, or every reason it
/// can't happen.
pub(super) fn facts(
    decision: &Decision,
    observation: &Observation,
    _choice: TargetChoice,
) -> Result<Facts, Refusal> {
    let item = &decision.item.id;
    let mut reasons = Vec::new();
    if decision.next_step.is_none() {
        reasons.push(RefusalReason::NoNextStep);
    }
    if !observed(item, observation) {
        reasons.push(RefusalReason::WorkItemGone);
        return Err(Refusal { reasons });
    }
    let repo = item.repo();
    let Some(repo_facts) = observation.repo(repo) else {
        reasons.push(RefusalReason::RepoUnknown { repo: repo.clone() });
        return Err(Refusal { reasons });
    };
    if repo_facts.is_archived {
        reasons.push(RefusalReason::RepoArchived { repo: repo.clone() });
    }
    let Some(step) = decision.next_step else {
        return Err(Refusal { reasons });
    };
    let start = match item {
        WorkItemId::Branch { name, .. } => {
            Some((BaseRef::HeadBranch(name.clone()), name.clone(), false))
        }
        WorkItemId::Issue { number, .. } => {
            let slug = names::issue_slug(*number, &decision.item.title);
            new_branch(repo_facts, step, &slug, &mut reasons)
        }
        WorkItemId::Run { workflow, .. } => {
            let slug = [decision.item.title.as_str(), workflow.0.as_str()]
                .into_iter()
                .map(names::slug)
                .find(|s| !s.is_empty())
                .unwrap_or_else(|| FALLBACK_WORKFLOW_SLUG.to_owned());
            new_branch(repo_facts, step, &slug, &mut reasons)
        }
        // PRs have their own facts (`plan::facts`); never routed here.
        WorkItemId::PullRequest { .. } => {
            reasons.push(RefusalReason::WorkItemGone);
            return Err(Refusal { reasons });
        }
    };
    let (Some((base, branch, fresh)), true) = (start, reasons.is_empty()) else {
        return Err(Refusal { reasons });
    };
    Ok(Facts {
        step,
        target: target(repo_facts),
        number: match item {
            WorkItemId::Issue { number, .. } => Some(*number),
            _ => None,
        },
        base,
        base_commit: None,
        branch,
        fresh,
    })
}

/// A new `<type>_<slug>` branch from the default branch; `None` (with the
/// reason) for an empty Repo.
fn new_branch(
    repo: &Repo,
    step: NextStep,
    slug: &str,
    reasons: &mut Vec<RefusalReason>,
) -> Option<(BaseRef, String, bool)> {
    let Some(default) = &repo.default_branch else {
        reasons.push(RefusalReason::NoDefaultBranch {
            repo: repo.id.clone(),
        });
        return None;
    };
    Some((
        BaseRef::DefaultBranch(default.clone()),
        names::new_branch_name(step, slug),
        true,
    ))
}

/// The Work Item is in the Observation den last made.
fn observed(item: &WorkItemId, observation: &Observation) -> bool {
    match item {
        WorkItemId::Issue { repo, number } => observation
            .issues
            .iter()
            .any(|i| &i.repo == repo && i.number == *number),
        WorkItemId::Branch { repo, name } => observation
            .branches
            .iter()
            .any(|b| &b.repo == repo && &b.name == name),
        WorkItemId::Run { repo, workflow } => observation
            .runs
            .iter()
            .any(|r| &r.repo == repo && &r.workflow == workflow),
        WorkItemId::PullRequest { repo, number } => observation
            .pull_requests
            .iter()
            .any(|p| &p.repo == repo && p.number == *number),
    }
}
