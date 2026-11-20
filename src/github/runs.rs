//! Runs on each Repo's default branch, through the REST API (`rest.rs`):
//! GraphQL has no listing of a branch's workflow Runs.
//!
//! Per Repo (not archived, not empty), cheapest first:
//!
//! 1. One page of the newest Runs on the default branch. A Repo without Actions,
//!    or whose default-branch Runs all fit on that page, costs this one call.
//! 2. Only when more Runs exist: the Repo's workflow list, then, for each
//!    workflow absent from the first page, that workflow's newest default-branch
//!    Runs. Without this, a busy workflow's Runs would push a quieter one's
//!    latest (possibly failed) Run off the page.
//!
//! The facts keep, per workflow, only the latest attempt of the newest Run that
//! ran on the default branch itself. Anything den skipped or could not read is
//! recorded as partial or failed coverage for that Repo. A rate limit, or
//! [`MAX_TIMED_OUT_REQUESTS_PER_OWNER`] timed-out requests, stops the Owner's
//! remaining requests and marks the Owner's Runs partial. The Owner's Runs are
//! never more complete than the list of Repos they were read from.

use super::{
    graphql::TransportError,
    rest::{self, Rest, RestResponse, encode},
    session::{Fetched, KindFetch, Session},
};
use crate::{
    domain::{
        CoverageEntry, Incompleteness, Repo, RepoId, Source, SourceKind, SourceState, WorkflowId,
        facts::{Run, RunConclusion, RunStatus},
    },
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    },
};

/// Runs per page of a Repo's default-branch Runs (GitHub's maximum).
pub const RUNS_PAGE_SIZE: usize = 100;
/// Workflows read per Repo (GitHub's maximum page); more is recorded as partial.
pub const MAX_WORKFLOWS_PER_REPO: usize = 100;
/// Runs read per workflow when looking up one workflow's latest default-branch Run.
pub const WORKFLOW_RUNS_PAGE_SIZE: usize = 10;
/// REST requests in flight at once for one Owner's Repos. Bounded to stay well
/// inside GitHub's secondary rate limits while keeping ~100 Repos to seconds.
pub const CONCURRENT_REQUESTS_PER_OWNER: usize = 8;
/// Timed-out REST requests one Owner's Runs survive. One more and den stops
/// reading them: a GitHub that slow would otherwise hold the survey for a full
/// `gh` timeout per remaining request.
pub const MAX_TIMED_OUT_REQUESTS_PER_OWNER: usize = 3;

/// What GitHub appends to a Run's workflow path when it names the ref the
/// workflow file was read from. Workflow files live directly in
/// `.github/workflows/`, so no directory in the path itself can contain it.
const REF_SUFFIX: &str = "@refs/";

/// Events whose Runs belong to a Pull Request, not to the branch they name.
const PULL_REQUEST_EVENTS: [&str; 4] = [
    "pull_request",
    "pull_request_target",
    "pull_request_review",
    "pull_request_review_comment",
];

#[derive(Deserialize)]
struct RunsPage {
    total_count: u64,
    #[serde(default)]
    workflow_runs: Vec<RunNode>,
}

#[derive(Deserialize)]
struct RunNode {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    workflow_id: u64,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    run_attempt: Option<u32>,
    html_url: String,
    head_sha: String,
    #[serde(default)]
    head_branch: Option<String>,
    event: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    actor: Option<Login>,
    #[serde(default)]
    triggering_actor: Option<Login>,
    #[serde(default)]
    head_repository: Option<FullName>,
    #[serde(default)]
    run_started_at: Option<Timestamp>,
    updated_at: Timestamp,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

#[derive(Deserialize)]
struct FullName {
    full_name: String,
}

#[derive(Deserialize)]
struct WorkflowsPage {
    total_count: u64,
    #[serde(default)]
    workflows: Vec<WorkflowNode>,
}

#[derive(Deserialize)]
struct WorkflowNode {
    id: u64,
    name: String,
}

/// A Repo whose Runs den reads, with its default branch.
struct Target<'r> {
    repo: &'r RepoId,
    default_branch: &'r str,
}

/// Shared by one Owner's workers: stop when GitHub rate-limits, keeps timing
/// out, or the survey cannot go on.
#[derive(Default)]
struct Stop {
    stopped: AtomicBool,
    rate_limited: AtomicBool,
    resets_at: Mutex<Option<Timestamp>>,
    /// Requests that got an HTTP response.
    responses: AtomicU32,
    timeouts: AtomicUsize,
    gave_up_on_timeouts: AtomicBool,
}

impl Stop {
    fn is_set(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn halt(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// Notes `state`; a rate limit stops the remaining requests.
    fn observe(&self, state: &SourceState) {
        if let SourceState::Partial(Incompleteness::RateLimited { resets_at }) = state {
            self.rate_limited.store(true, Ordering::Relaxed);
            if let (Some(at), Ok(mut kept)) = (resets_at, self.resets_at.lock()) {
                *kept = Some(kept.map_or(*at, |k| k.max(*at)));
            }
            self.halt();
        }
    }

    fn responded(&self) {
        self.responses.fetch_add(1, Ordering::Relaxed);
    }

    /// One more request timed out; past the bound the remaining requests stop.
    fn timed_out(&self) {
        let timeouts = self.timeouts.fetch_add(1, Ordering::Relaxed) + 1;
        if timeouts > MAX_TIMED_OUT_REQUESTS_PER_OWNER {
            self.gave_up_on_timeouts.store(true, Ordering::Relaxed);
            self.halt();
        }
    }

    fn rate_limit(&self) -> Option<SourceState> {
        self.rate_limited.load(Ordering::Relaxed).then(|| {
            SourceState::Partial(Incompleteness::RateLimited {
                resets_at: self.resets_at.lock().map_or(None, |at| *at),
            })
        })
    }

    /// Why den stopped early, for the Owner's Runs: `None` if it didn't.
    fn owner_state(&self) -> Option<SourceState> {
        self.rate_limit().or_else(|| {
            self.gave_up_on_timeouts.load(Ordering::Relaxed).then(|| {
                SourceState::Partial(Incompleteness::PaginationStopped {
                    pages_read: self.responses.load(Ordering::Relaxed),
                    reason: format!(
                        "more than {MAX_TIMED_OUT_REQUESTS_PER_OWNER} requests to GitHub timed out, so den stopped reading this Owner's Runs"
                    ),
                })
            })
        })
    }
}

/// The Owner's REST side, with every request's outcome noted on [`Stop`].
struct Watched<'a> {
    rest: &'a dyn Rest,
    stop: &'a Stop,
}

impl Rest for Watched<'_> {
    fn get(&self, path: &str) -> Result<RestResponse, TransportError> {
        let result = self.rest.get(path);
        match &result {
            Ok(_) => self.stop.responded(),
            Err(TransportError::TimedOut { .. }) => self.stop.timed_out(),
            Err(_) => {}
        }
        result
    }
}

/// Reads the default-branch Runs of `repos`. `repo_list` is how completely the
/// Owner's Repos were listed: Runs of Repos missing from the list weren't read.
pub fn fetch(
    session: &Session<'_>,
    owner: &str,
    repos: &[Repo],
    repo_list: &SourceState,
) -> Result<KindFetch<Run>, SurveyFailure> {
    let owner_source = Source::Owner {
        owner: owner.to_owned(),
        kind: SourceKind::Runs,
    };
    let Some(rest) = session.rest() else {
        return Ok(KindFetch {
            items: Vec::new(),
            coverage: vec![CoverageEntry {
                source: owner_source,
                state: SourceState::NotYetFetched,
            }],
        });
    };
    // Archived Repos can't run workflows, so nobody can fix their CI; empty
    // Repos have no default branch to run on.
    let targets: Vec<Target<'_>> = repos
        .iter()
        .filter(|r| !r.is_archived)
        .filter_map(|r| {
            r.default_branch.as_deref().map(|branch| Target {
                repo: &r.id,
                default_branch: branch,
            })
        })
        .collect();
    let stop = Stop::default();
    let watched = Watched { rest, stop: &stop };
    let rest: &dyn Rest = &watched;
    let mut progress: Vec<Progress> = targets.iter().map(|_| Progress::default()).collect();

    // Each phase spreads its requests over the same bounded pool, so one Repo
    // with many workflows doesn't serialise the Owner behind it.
    let first_pages = run_phase(&targets, &stop, |target| first_page(rest, target, &stop));
    let mut busy = Vec::new();
    for (index, result) in first_pages.into_iter().enumerate() {
        let Some(page) = result? else {
            continue;
        };
        let Some(repo) = progress.get_mut(index) else {
            continue;
        };
        repo.attempted = true;
        repo.note(page.state);
        repo.offer_all(page.facts);
        if page.more_runs_exist {
            busy.push(index);
        }
    }

    let lists = run_phase(&busy, &stop, |&index| {
        workflow_list(rest, &targets[index], &stop)
    });
    let mut lookups: Vec<(usize, WorkflowNode)> = Vec::new();
    for (&index, result) in busy.iter().zip(lists) {
        let Some(repo) = progress.get_mut(index) else {
            continue;
        };
        let Some((state, workflows)) = result? else {
            repo.note(stopped_early(&stop));
            continue;
        };
        repo.note(state);
        lookups.extend(
            workflows
                .into_iter()
                .filter(|w| !repo.latest.has_workflow(w.id))
                .map(|w| (index, w)),
        );
    }

    let found = run_phase(&lookups, &stop, |(index, workflow)| {
        workflow_runs(rest, &targets[*index], workflow, &stop)
    });
    for ((index, _), result) in lookups.iter().zip(found) {
        let Some(repo) = progress.get_mut(*index) else {
            continue;
        };
        let Some((facts, state)) = result? else {
            repo.note(stopped_early(&stop));
            continue;
        };
        repo.note(state);
        repo.offer_all(facts);
    }

    let mut out = KindFetch::default();
    for (target, repo) in targets.iter().zip(progress) {
        // Not attempted: den stopped the Owner early (recorded below).
        if !repo.attempted {
            continue;
        }
        out.items.extend(repo.latest.into_runs());
        if !repo.state.is_complete() {
            out.coverage.push(CoverageEntry {
                source: Source::Repo {
                    repo: target.repo.clone(),
                    kind: SourceKind::Runs,
                },
                state: repo.state,
            });
        }
    }
    let stopped = stop.owner_state().unwrap_or(SourceState::Complete);
    out.coverage.push(CoverageEntry {
        source: owner_source,
        // On a tie the stop wins: a rate limit carries its reset time.
        state: stopped.worst(super::of_listed_repos_only(repo_list, "Runs")),
    });
    Ok(out)
}

/// One Repo's Runs as the phases fill them in.
struct Progress {
    latest: Latest,
    state: SourceState,
    attempted: bool,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            latest: Latest::default(),
            state: SourceState::Complete,
            attempted: false,
        }
    }
}

impl Progress {
    fn note(&mut self, state: SourceState) {
        let previous = std::mem::replace(&mut self.state, SourceState::Complete);
        self.state = previous.worst(state);
    }

    fn offer_all(&mut self, facts: Vec<(u64, Run)>) {
        for fact in facts {
            self.latest.offer(fact);
        }
    }
}

/// Runs `work` over `items` on the Owner's bounded pool. Items left once
/// `stop` is set are skipped (`None`); a survey-stopping error sets it.
fn run_phase<T: Sync, R: Send>(
    items: &[T],
    stop: &Stop,
    work: impl Fn(&T) -> Result<R, SurveyFailure> + Sync,
) -> Vec<Result<Option<R>, SurveyFailure>> {
    bounded_parallel(items, CONCURRENT_REQUESTS_PER_OWNER, |item| {
        if stop.is_set() {
            return Ok(None);
        }
        let result = work(item);
        if result.is_err() {
            stop.halt();
        }
        result.map(Some)
    })
}

/// What the first page of a Repo's default-branch Runs gave.
struct FirstPage {
    facts: Vec<(u64, Run)>,
    state: SourceState,
    more_runs_exist: bool,
}

fn first_page(
    rest: &dyn Rest,
    target: &Target<'_>,
    stop: &Stop,
) -> Result<FirstPage, SurveyFailure> {
    let path = format!(
        "{}/actions/runs?branch={}&per_page={RUNS_PAGE_SIZE}&exclude_pull_requests=true",
        repo_path(target.repo),
        encode(target.default_branch),
    );
    Ok(match rest::get_json::<RunsPage>(rest, &path)? {
        Fetched::Data { data, state } => {
            let more_runs_exist = more_than(data.total_count, data.workflow_runs.len());
            FirstPage {
                facts: facts(data, target),
                state,
                more_runs_exist,
            }
        }
        Fetched::Lost(state) => {
            stop.observe(&state);
            FirstPage {
                facts: Vec::new(),
                state,
                more_runs_exist: false,
            }
        }
    })
}

/// The Repo's workflows, with the coverage state of reading them.
fn workflow_list(
    rest: &dyn Rest,
    target: &Target<'_>,
    stop: &Stop,
) -> Result<(SourceState, Vec<WorkflowNode>), SurveyFailure> {
    let path = format!(
        "{}/actions/workflows?per_page={MAX_WORKFLOWS_PER_REPO}",
        repo_path(target.repo)
    );
    Ok(match rest::get_json::<WorkflowsPage>(rest, &path)? {
        Fetched::Data { data, state } => {
            let state = if more_than(data.total_count, data.workflows.len()) {
                state.worst(stopped_after_first_page(format!(
                    "{} workflows; den reads at most {MAX_WORKFLOWS_PER_REPO}",
                    data.total_count
                )))
            } else {
                state
            };
            (state, data.workflows)
        }
        Fetched::Lost(lost) => {
            stop.observe(&lost);
            (
                beyond_first_page(
                    lost,
                    "more Runs than one page, and the workflow list could not be read",
                ),
                Vec::new(),
            )
        }
    })
}

/// One workflow's newest default-branch Runs.
fn workflow_runs(
    rest: &dyn Rest,
    target: &Target<'_>,
    workflow: &WorkflowNode,
    stop: &Stop,
) -> Result<(Vec<(u64, Run)>, SourceState), SurveyFailure> {
    let path = format!(
        "{}/actions/workflows/{}/runs?branch={}&per_page={WORKFLOW_RUNS_PAGE_SIZE}&exclude_pull_requests=true",
        repo_path(target.repo),
        workflow.id,
        encode(target.default_branch),
    );
    Ok(match rest::get_json::<RunsPage>(rest, &path)? {
        Fetched::Data { data, state } => {
            let more = more_than(data.total_count, data.workflow_runs.len());
            let facts = facts(data, target);
            let state = if more && !facts.iter().any(|(_, run)| run.on_default_branch) {
                state.worst(stopped_after_first_page(format!(
                    "none of the newest {WORKFLOW_RUNS_PAGE_SIZE} Runs of workflow {} ran on the default branch itself",
                    workflow.name
                )))
            } else {
                state
            };
            (facts, state)
        }
        Fetched::Lost(lost) => {
            stop.observe(&lost);
            (
                Vec::new(),
                beyond_first_page(
                    lost,
                    &format!("the Runs of workflow {} could not be read", workflow.name),
                ),
            )
        }
    })
}

fn facts(page: RunsPage, target: &Target<'_>) -> Vec<(u64, Run)> {
    page.workflow_runs
        .into_iter()
        .map(|node| node.into_fact(target))
        .collect()
}

fn repo_path(repo: &RepoId) -> String {
    format!("repos/{}/{}", encode(&repo.owner), encode(&repo.name))
}

/// GitHub counted more than it sent.
fn more_than(total_count: u64, received: usize) -> bool {
    usize::try_from(total_count).map_or(true, |total| total > received)
}

/// A request after the first page was lost: a rate limit stays a rate limit
/// (with its reset time); anything else makes the Repo partial, saying what.
fn beyond_first_page(lost: SourceState, what: &str) -> SourceState {
    match lost {
        limited @ SourceState::Partial(Incompleteness::RateLimited { .. }) => limited,
        other => stopped_after_first_page(format!("{what} ({})", other.describe())),
    }
}

/// A Repo whose later requests were skipped because the Owner stopped.
fn stopped_early(stop: &Stop) -> SourceState {
    stop.rate_limit().unwrap_or_else(|| {
        stopped_after_first_page(
            if stop.gave_up_on_timeouts.load(Ordering::Relaxed) {
                "den stopped reading this Owner's Runs after repeated timeouts"
            } else {
                "den stopped reading this Repo's Runs early"
            }
            .to_owned(),
        )
    })
}

/// Partial: the first page was read, and `reason` says what was not.
fn stopped_after_first_page(reason: String) -> SourceState {
    SourceState::Partial(Incompleteness::PaginationStopped {
        pages_read: 1,
        reason,
    })
}

/// The latest attempt of the newest default-branch Run per workflow.
#[derive(Default)]
struct Latest {
    by_workflow: BTreeMap<u64, Run>,
}

impl Latest {
    fn offer(&mut self, (workflow_id, run): (u64, Run)) {
        if !run.on_default_branch {
            return;
        }
        match self.by_workflow.get(&workflow_id) {
            Some(kept) if (kept.run_id, kept.attempt) >= (run.run_id, run.attempt) => {}
            _ => {
                self.by_workflow.insert(workflow_id, run);
            }
        }
    }

    fn has_workflow(&self, workflow_id: u64) -> bool {
        self.by_workflow.contains_key(&workflow_id)
    }

    fn into_runs(self) -> Vec<Run> {
        self.by_workflow.into_values().collect()
    }
}

impl RunNode {
    /// The fact, with GitHub's numeric workflow ID beside it for bookkeeping.
    fn into_fact(self, target: &Target<'_>) -> (u64, Run) {
        let repo = target.repo;
        // A Run whose source Repo GitHub didn't name (a deleted fork, say) is
        // not known to have run on this Repo's default branch, so it doesn't count.
        let same_repo = self
            .head_repository
            .as_ref()
            .is_some_and(|h| h.full_name.eq_ignore_ascii_case(&repo.to_string()));
        let on_default_branch = same_repo
            && self.head_branch.as_deref() == Some(target.default_branch)
            && !PULL_REQUEST_EVENTS.contains(&self.event.as_str());
        // A Run's path may carry the ref it ran from (`ci.yml@refs/heads/main`).
        // Only that suffix goes: a workflow file name may itself contain `@`.
        let workflow = self
            .path
            .as_deref()
            .map(|p| {
                p.split_once(REF_SUFFIX)
                    .map_or(p, |(path, _)| path)
                    .to_owned()
            })
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| format!("workflow {}", self.workflow_id));
        let run = Run {
            repo: repo.clone(),
            workflow_name: self.name.unwrap_or_else(|| workflow.clone()),
            workflow: WorkflowId(workflow),
            run_id: self.id,
            attempt: self.run_attempt.unwrap_or(1),
            url: self.html_url,
            head_commit: self.head_sha,
            head_branch: self.head_branch,
            event: self.event,
            triggering_actor: self.triggering_actor.map(|a| a.login),
            actor: self.actor.map(|a| a.login),
            status: self
                .status
                .as_deref()
                .map_or(RunStatus::Unknown(String::new()), RunStatus::from_github),
            conclusion: self.conclusion.as_deref().map(RunConclusion::from_github),
            on_default_branch,
            started_at: self.run_started_at,
            updated_at: self.updated_at,
        };
        (self.workflow_id, run)
    }
}

/// Runs `work` over `items` on at most `workers` threads, results in item order.
/// A worker that panics yields [`SurveyFailure::Interrupted`] for its items.
fn bounded_parallel<T, R>(
    items: &[T],
    workers: usize,
    work: impl Fn(&T) -> Result<R, SurveyFailure> + Sync,
) -> Vec<Result<R, SurveyFailure>>
where
    T: Sync,
    R: Send,
{
    let next = AtomicUsize::new(0);
    let mut slots: Vec<Option<Result<R, SurveyFailure>>> = Vec::new();
    slots.resize_with(items.len(), || None);
    let finished: Vec<Vec<(usize, Result<R, SurveyFailure>)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers.clamp(1, items.len().max(1)))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else {
                            break;
                        };
                        done.push((index, work(item)));
                    }
                    done
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for (index, result) in finished.into_iter().flatten() {
        if let Some(slot) = slots.get_mut(index) {
            *slot = Some(result);
        }
    }
    slots
        .into_iter()
        .map(|slot| {
            slot.unwrap_or_else(|| {
                Err(SurveyFailure::Interrupted {
                    detail: "reading a Repo's Runs stopped unexpectedly".to_owned(),
                })
            })
        })
        .collect()
}
