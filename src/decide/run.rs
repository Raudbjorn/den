//! Run rules (spec, "Next Step rules"): the latest Run of a workflow on the
//! default branch failed → Fix CI, Waiting On the Run's triggering actor.
//!
//! Each workflow is judged on its own: per Repo and workflow, only the newest
//! Run counts, and of that Run only its latest attempt. So a successful retry
//! or a newer successful Run clears an older failure, and a passing workflow
//! never hides a failing one. Runs off the default branch (Pull Request runs,
//! other branches) never count. A workflow whose latest Run did not fail is not
//! a Work Item at all.

use super::{Attribution, Context, Decision, Explanation, WorkItem};
use crate::domain::{
    NextStep, RepoId, WorkflowId,
    facts::{Run, RunConclusion, RunStatus},
};
use std::collections::BTreeMap;

/// How many characters of a commit SHA the Explanation shows.
const SHORT_SHA_CHARS: usize = 7;
/// GitHub numbers a Run's attempts from 1; later attempts are retries.
const FIRST_ATTEMPT: u32 = 1;

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    latest_per_workflow(&ctx.observation.runs)
        .into_iter()
        .filter_map(|run| decide_one(ctx, run))
        .collect()
}

/// The latest attempt of the newest Run of each workflow on a default branch,
/// in the order each workflow first appears in the facts. "Newest" is the
/// highest Run ID: GitHub assigns them in increasing order and a re-run keeps
/// its ID, so a retry never makes an older Run look newer.
fn latest_per_workflow(runs: &[Run]) -> Vec<&Run> {
    let mut first_seen: Vec<(&RepoId, &WorkflowId)> = Vec::new();
    let mut latest: BTreeMap<(&RepoId, &WorkflowId), &Run> = BTreeMap::new();
    for run in runs.iter().filter(|r| r.on_default_branch) {
        let key = (&run.repo, &run.workflow);
        match latest.get(&key) {
            Some(kept) if (kept.run_id, kept.attempt) >= (run.run_id, run.attempt) => {}
            Some(_) => {
                latest.insert(key, run);
            }
            None => {
                first_seen.push(key);
                latest.insert(key, run);
            }
        }
    }
    first_seen
        .into_iter()
        .filter_map(|key| latest.get(&key).copied())
        .collect()
}

/// Fix CI when the Run completed and failed; otherwise no Work Item.
fn decide_one(ctx: &Context<'_>, run: &Run) -> Option<Decision> {
    if run.status != RunStatus::Completed {
        return None;
    }
    let conclusion = run.conclusion.as_ref()?;
    if !is_failure(conclusion) {
        return None;
    }
    let (login, role) = match (&run.triggering_actor, &run.actor) {
        (Some(login), _) => (Some(login.as_str()), "the Run's triggering actor"),
        // On a first attempt whoever started the Run triggered it. A retry may
        // have been started by someone else, so the first attempt's actor says
        // nothing about who owes the retry: that stays unattributed.
        (None, Some(login)) if run.attempt == FIRST_ATTEMPT => {
            (Some(login.as_str()), "the actor who started the Run")
        }
        _ => (None, "the Run's triggering actor"),
    };
    let attribution = Attribution::person(ctx.viewers.github, login, role);
    let mut unknowns = Vec::new();
    if login.is_none() {
        unknowns.push(match &run.actor {
            Some(started_by) => format!(
                "who triggered attempt {} (GitHub reported only who started the first attempt: {started_by})",
                run.attempt
            ),
            None => "who triggered the Run (GitHub reported no actor)".to_owned(),
        });
    }
    let explanation = Explanation {
        winning_rule: Some(format!(
            "the latest Run of workflow {} on the default branch {}",
            run.workflow_name,
            conclusion.describe()
        )),
        no_step_reason: None,
        other_conditions: other_conditions(run),
        unknowns,
    };
    let item = WorkItem {
        id: run.id(),
        identifier: None,
        title: run.workflow_name.clone(),
        url: run.url.clone(),
        // When the failing attempt ran; GitHub's own `updated_at` drifts.
        updated_at: run.started_at.unwrap_or(run.updated_at),
    };
    Some(Decision::new(
        ctx,
        item,
        Some(NextStep::FixCi),
        attribution,
        explanation,
    ))
}

/// Conclusions that mean the workflow is broken. A cancelled, skipped or
/// approval-waiting Run is not a failure: nothing was shown to be wrong.
fn is_failure(conclusion: &RunConclusion) -> bool {
    matches!(
        conclusion,
        RunConclusion::Failure | RunConclusion::TimedOut | RunConclusion::StartupFailure
    )
}

fn other_conditions(run: &Run) -> Vec<String> {
    let commit: String = run.head_commit.chars().take(SHORT_SHA_CHARS).collect();
    let branch = run.head_branch.as_deref().unwrap_or("the default branch");
    let mut conditions = vec![
        format!("Run {}, attempt {}", run.run_id, run.attempt),
        format!("triggered by a {} event on {branch} at {commit}", run.event),
    ];
    if let (Some(started_by), Some(retried_by)) = (&run.actor, &run.triggering_actor)
        && !started_by.eq_ignore_ascii_case(retried_by)
    {
        conditions.push(format!(
            "first started by {started_by}, this attempt by {retried_by}"
        ));
    }
    conditions
}
