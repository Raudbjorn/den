//! Run builder (ticket #7). A built Run is the latest attempt of a completed,
//! successful Run of workflow `ci.yml` on `main`, the Repo's default branch.

use super::{repo_id, ts};
use den::domain::{
    WorkflowId,
    facts::{Run, RunConclusion, RunStatus},
};

/// When a built Run was last updated, unless set.
const DEFAULT_UPDATED: &str = "2026-11-18T12:00:00Z";

/// Run `run_id` of workflow `workflow` (a file name under `.github/workflows/`)
/// in `repo` ("owner/name"), triggered by "someone" pushing to `main`.
pub fn run(repo: &str, workflow: &str, run_id: u64) -> RunBuilder {
    let repo = repo_id(repo);
    RunBuilder {
        run: Run {
            url: format!("https://github.com/{repo}/actions/runs/{run_id}"),
            repo,
            workflow: WorkflowId(format!(".github/workflows/{workflow}")),
            workflow_name: workflow.trim_end_matches(".yml").to_owned(),
            run_id,
            attempt: 1,
            head_commit: format!("{run_id:040x}"),
            head_branch: Some("main".to_owned()),
            event: "push".to_owned(),
            triggering_actor: Some("someone".to_owned()),
            actor: Some("someone".to_owned()),
            status: RunStatus::Completed,
            conclusion: Some(RunConclusion::Success),
            on_default_branch: true,
            started_at: Some(ts(DEFAULT_UPDATED)),
            updated_at: ts(DEFAULT_UPDATED),
        },
    }
}

pub struct RunBuilder {
    run: Run,
}

impl RunBuilder {
    /// The workflow's display name (by default its file name without `.yml`).
    pub fn named(mut self, name: &str) -> Self {
        self.run.workflow_name = name.to_owned();
        self
    }

    pub fn failed(self) -> Self {
        self.concluded(RunConclusion::Failure)
    }

    pub fn concluded(mut self, conclusion: RunConclusion) -> Self {
        self.run.status = RunStatus::Completed;
        self.run.conclusion = Some(conclusion);
        self
    }

    /// Still running: no conclusion yet.
    pub fn in_progress(mut self) -> Self {
        self.run.status = RunStatus::InProgress;
        self.run.conclusion = None;
        self
    }

    pub fn attempt(mut self, attempt: u32) -> Self {
        self.run.attempt = attempt;
        self
    }

    /// Triggered (this attempt and the first) by `login`.
    pub fn by(mut self, login: &str) -> Self {
        self.run.triggering_actor = Some(login.to_owned());
        self.run.actor = Some(login.to_owned());
        self
    }

    /// This attempt was re-run by `login`; the first attempt keeps its actor.
    pub fn retried_by(mut self, login: &str) -> Self {
        self.run.triggering_actor = Some(login.to_owned());
        self
    }

    /// GitHub reported who started the first attempt (`login`) but no
    /// triggering actor for this attempt.
    pub fn started_by_only(mut self, login: &str) -> Self {
        self.run.triggering_actor = None;
        self.run.actor = Some(login.to_owned());
        self
    }

    /// GitHub reported no triggering actor (and no actor at all).
    pub fn no_actor(mut self) -> Self {
        self.run.triggering_actor = None;
        self.run.actor = None;
        self
    }

    /// Ran on `branch` by `event`, not on the default branch.
    pub fn off_default_branch(mut self, branch: &str, event: &str) -> Self {
        self.run.head_branch = Some(branch.to_owned());
        self.run.event = event.to_owned();
        self.run.on_default_branch = false;
        self
    }

    pub fn event(mut self, event: &str) -> Self {
        self.run.event = event.to_owned();
        self
    }

    /// This attempt started at `at`.
    pub fn started(mut self, at: &str) -> Self {
        self.run.started_at = Some(ts(at));
        self
    }

    /// GitHub's `updated_at` (which drifts after the Run ends) is `at`, and it
    /// sent no start time.
    pub fn only_updated(mut self, at: &str) -> Self {
        self.run.started_at = None;
        self.run.updated_at = ts(at);
        self
    }

    pub fn build(self) -> Run {
        self.run
    }
}
