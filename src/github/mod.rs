//! The GitHub source: the real [`GitHubSource`](crate::ports::GitHubSource),
//! reading through `gh api graphql`.
//!
//! One fetcher per Work Item kind (`pull_requests`, `issues`, `branches`, `runs`),
//! each with its queries in `queries/`. [`survey`] runs the viewer, then for each
//! Owner its Repos and every kind, and collects facts and coverage into one
//! Observation. A kind ticket edits only its own fetcher.

pub mod branches;
pub mod graphql;
pub mod issues;
pub mod pull_requests;
pub mod repos;
pub mod runs;
pub mod session;
pub mod viewer;

use crate::{
    domain::{
        CoverageEntry, Observation, Repo,
        facts::{Branch, Issue, PullRequest, Run},
    },
    ports::{GitHubSource, SurveyFailure, SurveyRequest},
};
use graphql::GraphQl;
use jiff::Timestamp;
use session::{RateLimitMemo, Session};

/// Surveys the Owners. `observed_at` is when the survey started: every fact is at
/// least that fresh. Owners are read in parallel (one thread each; the config
/// bounds how many) and merged in config order, so the result is deterministic.
pub fn survey(
    gql: &dyn GraphQl,
    owners: &[String],
    observed_at: Timestamp,
) -> Result<Observation, SurveyFailure> {
    let rate_limit = RateLimitMemo::default();
    let (viewer, viewer_coverage) = viewer::fetch(&Session::new(gql, &rate_limit))?;
    let mut observation = Observation::empty(observed_at, viewer, owners.to_vec());
    record(&mut observation, [viewer_coverage]);
    let per_owner: Vec<Result<OwnerSurvey, SurveyFailure>> = std::thread::scope(|scope| {
        let handles: Vec<_> = owners
            .iter()
            .map(|owner| {
                let rate_limit = &rate_limit;
                scope.spawn(move || survey_owner(&Session::new(gql, rate_limit), owner))
            })
            .collect();
        handles
            .into_iter()
            .zip(owners)
            .map(|(handle, owner)| {
                handle.join().unwrap_or_else(|_| {
                    Err(SurveyFailure::Interrupted {
                        detail: format!("the survey of {owner} stopped unexpectedly"),
                    })
                })
            })
            .collect()
    });
    for owner in per_owner {
        let owner = owner?;
        record(&mut observation, owner.coverage);
        observation.repos.extend(owner.repos);
        observation.pull_requests.extend(owner.pull_requests);
        observation.issues.extend(owner.issues);
        observation.branches.extend(owner.branches);
        observation.runs.extend(owner.runs);
    }
    for entry in observation.coverage.entries() {
        tracing::info!(source = %entry.source.label(), state = %entry.state.describe(), "coverage");
    }
    Ok(observation)
}

/// Everything read for one Owner.
struct OwnerSurvey {
    repos: Vec<Repo>,
    pull_requests: Vec<PullRequest>,
    issues: Vec<Issue>,
    branches: Vec<Branch>,
    runs: Vec<Run>,
    coverage: Vec<CoverageEntry>,
}

fn survey_owner(session: &Session<'_>, owner: &str) -> Result<OwnerSurvey, SurveyFailure> {
    let repos = repos::fetch(session, owner)?;
    let pull_requests = pull_requests::fetch(session, owner)?;
    let issues = issues::fetch(session, owner, &repos.items)?;
    let branches = branches::fetch(session, owner, &repos.items)?;
    let runs = runs::fetch(session, owner, &repos.items)?;
    let coverage = [
        repos.coverage,
        pull_requests.coverage,
        issues.coverage,
        branches.coverage,
        runs.coverage,
    ]
    .concat();
    Ok(OwnerSurvey {
        repos: repos.items,
        pull_requests: pull_requests.items,
        issues: issues.items,
        branches: branches.items,
        runs: runs.items,
        coverage,
    })
}

fn record(observation: &mut Observation, entries: impl IntoIterator<Item = CoverageEntry>) {
    for entry in entries {
        observation.coverage.record(entry.source, entry.state);
    }
}

/// The real GitHub source.
pub struct GhSource<G: GraphQl> {
    gql: G,
}

impl<G: GraphQl> GhSource<G> {
    pub fn new(gql: G) -> Self {
        Self { gql }
    }
}

impl<G: GraphQl> GitHubSource for GhSource<G> {
    fn survey(&self, request: &SurveyRequest) -> Result<Observation, SurveyFailure> {
        survey(&self.gql, &request.owners, Timestamp::now())
    }
}
