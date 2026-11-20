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

use super::{
    BaseRef, Refusal, RefusalReason, Target, TargetChoice, landing::LandingGroupPlan, names,
};
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
    pub new_branch: bool,
    /// Land (#40): the Landing Group the Job lands. `None` for every other kind.
    pub landing: Option<LandingGroupPlan>,
}

/// The target a choice names, and whether it is archived.
pub(super) struct ChosenTarget {
    pub target: Target,
    pub archived: bool,
}

/// The target `choice` names for a Work Item in `repo`: the Repo itself, or
/// (for a fork) its Upstream (#13). Never a silent substitute. The single
/// place a choice becomes a Target.
pub(super) fn chosen_target(
    repo: &Repo,
    choice: TargetChoice,
) -> Result<ChosenTarget, RefusalReason> {
    let (target, upstream_of) = match choice {
        TargetChoice::OwnRepo => (repo, None),
        TargetChoice::Upstream if !repo.is_fork => {
            return Err(RefusalReason::NotAFork {
                repo: repo.id.clone(),
            });
        }
        TargetChoice::Upstream => match repo.upstream() {
            Some(upstream) => (upstream, Some(repo.id.clone())),
            None => {
                return Err(RefusalReason::UpstreamUnknown {
                    fork: repo.id.clone(),
                });
            }
        },
    };
    Ok(ChosenTarget {
        target: Target {
            repo: target.id.clone(),
            url: target.url.clone(),
            is_fork: target.is_fork,
            upstream_of,
            default_branch: target.default_branch.clone(),
        },
        archived: target.is_archived,
    })
}

/// The facts for launching an Issue, a Branch or a Run, or every reason it
/// can't happen.
pub(super) fn facts(
    decision: &Decision,
    observation: &Observation,
    choice: TargetChoice,
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
    let Some(repo) = item.repo() else {
        reasons.push(RefusalReason::LinearIssueNotLaunchable);
        return Err(Refusal { reasons });
    };
    let Some(repo_facts) = observation.repo(repo) else {
        reasons.push(RefusalReason::RepoUnknown { repo: repo.clone() });
        return Err(Refusal { reasons });
    };
    let target = match chosen_target(repo_facts, choice) {
        Ok(target) => target,
        Err(reason) => {
            reasons.push(reason);
            return Err(Refusal { reasons });
        }
    };
    if target.archived {
        reasons.push(RefusalReason::RepoArchived {
            repo: target.target.repo.clone(),
        });
    }
    let Some(step) = decision.next_step else {
        return Err(Refusal { reasons });
    };
    let start = match item {
        // A Branch's commits live in the Repo it is in; the same name in
        // another target (a fork's Upstream) is another branch.
        WorkItemId::Branch { name, .. } if &target.target.repo != repo => {
            reasons.push(RefusalReason::BranchNotInTarget {
                repo: repo.clone(),
                branch: name.clone(),
                target: target.target.repo.clone(),
            });
            None
        }
        WorkItemId::Branch { name, .. } => {
            Some((BaseRef::HeadBranch(name.clone()), name.clone(), false))
        }
        WorkItemId::Issue { number, .. } => {
            let slug = names::issue_slug(*number, &decision.item.title);
            new_branch(&target.target, step, &slug, &mut reasons)
        }
        WorkItemId::Run { workflow, .. } => {
            let slug = [decision.item.title.as_str(), workflow.0.as_str()]
                .into_iter()
                .map(names::slug)
                .find(|s| !s.is_empty())
                .unwrap_or_else(|| FALLBACK_WORKFLOW_SLUG.to_owned());
            new_branch(&target.target, step, &slug, &mut reasons)
        }
        // PRs have their own facts (`plan::facts`); never routed here.
        WorkItemId::PullRequest { .. } => {
            reasons.push(RefusalReason::WorkItemGone);
            return Err(Refusal { reasons });
        }
        // Refused above: a Linear Issue has no Repo.
        WorkItemId::LinearIssue { .. } => {
            reasons.push(RefusalReason::LinearIssueNotLaunchable);
            return Err(Refusal { reasons });
        }
    };
    let (Some((base, branch, new_branch)), true) = (start, reasons.is_empty()) else {
        return Err(Refusal { reasons });
    };
    Ok(Facts {
        step,
        target: target.target,
        number: match item {
            WorkItemId::Issue { number, .. } => Some(*number),
            _ => None,
        },
        base,
        base_commit: None,
        branch,
        new_branch,
        landing: None,
    })
}

/// A new `<type>_<slug>` branch from the target's default branch (the
/// Upstream's when it is the target); `None` (with the reason) for an empty Repo.
fn new_branch(
    target: &Target,
    step: NextStep,
    slug: &str,
    reasons: &mut Vec<RefusalReason>,
) -> Option<(BaseRef, String, bool)> {
    let Some(default) = &target.default_branch else {
        reasons.push(RefusalReason::NoDefaultBranch {
            repo: target.repo.clone(),
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
        WorkItemId::LinearIssue { issue, .. } => {
            observation.linear.issues.iter().any(|i| &i.id == issue)
        }
    }
}
