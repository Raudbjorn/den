//! The GitHub source: the real [`GitHubSource`](crate::ports::GitHubSource),
//! reading through `gh api graphql`.
//!
//! One fetcher per Work Item kind (`pull_requests`, `issues`, `branches`, `runs`),
//! each with its queries in `queries/`. [`survey`] runs the viewer, then for each
//! Owner its Repos and every kind, and collects facts and coverage into one
//! Observation. A kind ticket edits only its own fetcher. `member_states`
//! reads, after the survey, the states of Landing Group members that weren't
//! among the open Pull Requests (#37).

pub mod branches;
pub mod graphql;
pub mod issues;
pub mod member_states;
pub mod pull_requests;
pub mod repos;
pub mod rest;
pub mod runs;
pub mod session;
pub mod viewer;

use crate::{
    domain::{
        Comparison, CoverageEntry, Observation, Repo, RepoId, Source, SourceKind, SourceState,
        UpstreamTargets, comparison_for,
        facts::{Branch, ComparisonBranch, Issue, PullRequest, Run},
    },
    ports::{GitHubSource, MemberStatesRead, MemberStatesRequest, SurveyFailure, SurveyRequest},
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
    survey_with(gql, owners, &UpstreamTargets::default(), observed_at)
}

/// [`survey`], comparing the Branches of the forks in `upstreams` against their
/// Upstream's default branch (#13) instead of their own.
pub fn survey_with(
    gql: &dyn GraphQl,
    owners: &[String],
    upstreams: &UpstreamTargets,
    observed_at: Timestamp,
) -> Result<Observation, SurveyFailure> {
    let request = SurveyRequest {
        owners: owners.to_vec(),
        upstreams: upstreams.clone(),
        screenshot_repos: Vec::new(),
    };
    survey_request(gql, &request, observed_at)
}

/// The survey `request` asks for: its Owners, its forks' Upstream targets
/// (#13), and the Repos whose PR bodies are read for screenshots (#79).
pub fn survey_request(
    gql: &dyn GraphQl,
    request: &SurveyRequest,
    observed_at: Timestamp,
) -> Result<Observation, SurveyFailure> {
    let owners = request.owners.as_slice();
    let rate_limit = RateLimitMemo::default();
    let (viewer, viewer_coverage) = viewer::fetch(&Session::new(gql, &rate_limit))?;
    let mut observation = Observation::empty(observed_at, viewer, owners.to_vec());
    record(&mut observation, [viewer_coverage]);
    let per_owner: Vec<Result<OwnerSurvey, SurveyFailure>> = std::thread::scope(|scope| {
        let handles: Vec<_> = owners
            .iter()
            .map(|owner| {
                let rate_limit = &rate_limit;
                let viewer = observation.viewer.login.as_str();
                scope.spawn(move || {
                    survey_owner(&Session::new(gql, rate_limit), owner, viewer, request)
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(owners)
            .map(|(handle, owner)| joined(handle.join(), owner))
            .collect()
    });
    for (owner, login) in per_owner.into_iter().zip(owners) {
        let owner = owner?;
        record(&mut observation, owner.coverage);
        // Each listed Repo's sources become its own (spec 0003, "Coverage
        // per Repo"); the Owner's entries stay for per-Owner kinds.
        let listed: Vec<RepoId> = owner.repos.iter().map(|r| r.id.clone()).collect();
        observation.coverage.split_by_repo(login, &listed);
        observation.repos.extend(owner.repos);
        observation.pull_requests.extend(owner.pull_requests);
        observation.issues.extend(owner.issues);
        observation.branches.extend(owner.branches);
        observation.runs.extend(owner.runs);
    }
    log_coverage(&observation);
    Ok(observation)
}

/// A Repo Survey (spec 0003, ADR-0005) of `repo`, with no PR bodies read for
/// screenshots. [`survey_repo_with`] says what it reads.
pub fn survey_repo(
    gql: &dyn GraphQl,
    repo: &RepoId,
    upstreams: &UpstreamTargets,
    observed_at: Timestamp,
) -> Result<Observation, SurveyFailure> {
    survey_repo_with(gql, repo, upstreams, &[], observed_at)
}

/// A Repo Survey (spec 0003, ADR-0005): the viewer, then `repo` and its open
/// Pull Requests, Issues, Branches and default-branch Runs, read at once.
/// Facts carry the Repo as GitHub spells it, and every source is recorded for
/// the Repo itself, never its Owner, so the cache merges it beside the
/// Owner's other Repos. It covers no Owner (`owners` is empty). A fork
/// targeting its Upstream compares its Branches there, as an Owner survey
/// does. PR bodies are read for screenshots when `repo` is one of
/// `screenshot_repos` (#79). `observed_at` is when it started.
pub fn survey_repo_with(
    gql: &dyn GraphQl,
    repo: &RepoId,
    upstreams: &UpstreamTargets,
    screenshot_repos: &[RepoId],
    observed_at: Timestamp,
) -> Result<Observation, SurveyFailure> {
    let rate_limit = RateLimitMemo::default();
    let session = Session::new(gql, &rate_limit);
    let (viewer, viewer_coverage) = viewer::fetch(&session)?;
    let mut observation = Observation::empty(observed_at, viewer, Vec::new());
    record(&mut observation, [viewer_coverage]);
    let (found, listing) = repos::fetch_one(&session, repo)?;
    let Some(found) = found else {
        record(&mut observation, [listing]);
        for kind in SourceKind::WORK_ITEMS {
            let source = Source::Repo {
                repo: repo.clone(),
                kind,
            };
            observation
                .coverage
                .record(source, SourceState::NotYetFetched);
        }
        log_coverage(&observation);
        return Ok(observation);
    };
    let id = &found.id;
    let listed = std::slice::from_ref(&found);
    let comparison = |r: &Repo| comparison_branch(r, upstreams);
    let comparison = &comparison;
    let rate_limit = &rate_limit;
    let viewer = observation.viewer.login.as_str();
    let (pull_requests, issues, branches, runs) = std::thread::scope(|scope| {
        let prs = scope.spawn(move || {
            let session = Session::new(gql, rate_limit);
            pull_requests::fetch_repo(&session, id, viewer, screenshot_repos)
        });
        let issues = scope.spawn(move || issues::fetch_one(&Session::new(gql, rate_limit), id));
        let branches = scope.spawn(move || {
            let session = Session::new(gql, rate_limit);
            branches::fetch_against(&session, &id.owner, listed, comparison)
        });
        let runs =
            scope.spawn(move || runs::fetch(&Session::new(gql, rate_limit), &id.owner, listed));
        (
            joined(prs.join(), id),
            joined(issues.join(), id),
            joined(branches.join(), id),
            joined(runs.join(), id),
        )
    });
    let (pull_requests, issues, branches, runs) = (pull_requests?, issues?, branches?, runs?);
    let listing_state = listing.state.clone();
    let entries = [
        vec![listing],
        pull_requests.coverage,
        issues.coverage,
        branches.coverage,
        runs.coverage,
    ]
    .concat();
    // The fetchers record what spans an Owner as the Owner's; here that
    // Owner is this one Repo.
    for entry in entries {
        let source = match entry.source {
            Source::Owner { kind, .. } => Source::Repo {
                repo: found.id.clone(),
                kind,
            },
            other => other,
        };
        observation.coverage.record(source, entry.state);
    }
    // A kind is never more complete than the Repo's listing, as for an Owner's
    // Repo list (`Coverage::record`): an error beside the Repo's data may have
    // nulled what a kind needs (its default branch), and reading nothing then
    // is not "there are none".
    if !listing_state.is_complete() {
        for kind in SourceKind::WORK_ITEMS {
            let source = Source::Repo {
                repo: found.id.clone(),
                kind,
            };
            observation.coverage.record(source, listing_state.clone());
        }
    }
    observation.pull_requests = pull_requests.items;
    observation.issues = issues.items;
    observation.branches = branches.items;
    observation.runs = runs.items;
    observation.repos.push(found);
    log_coverage(&observation);
    Ok(observation)
}

/// A survey's read that panicked stops the survey: `what` names what was
/// being read (an Owner, or the Repo of a Repo Survey).
fn joined<T>(
    joined: std::thread::Result<Result<T, SurveyFailure>>,
    what: &dyn std::fmt::Display,
) -> Result<T, SurveyFailure> {
    joined.unwrap_or_else(|_| {
        Err(SurveyFailure::Interrupted {
            detail: format!("the survey of {what} stopped unexpectedly"),
        })
    })
}

/// Logs a finished survey's coverage, one line per source.
fn log_coverage(observation: &Observation) {
    for entry in observation.coverage.entries() {
        tracing::info!(source = %entry.source.label(), state = %entry.state.describe(), "coverage");
    }
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

/// `viewer` is the GitHub viewer's login, for the PRs' Review Point facts.
fn survey_owner(
    session: &Session<'_>,
    owner: &str,
    viewer: &str,
    request: &SurveyRequest,
) -> Result<OwnerSurvey, SurveyFailure> {
    let upstreams = &request.upstreams;
    let repos = repos::fetch(session, owner)?;
    let pull_requests = pull_requests::fetch(session, owner, viewer, &request.screenshot_repos)?;
    let issues = issues::fetch(session, owner, &repos.items)?;
    let comparison = |repo: &Repo| comparison_branch(repo, upstreams);
    let branches = branches::fetch_against(session, owner, &repos.items, &comparison)?;
    // Each kind is never more complete than the Repo list: Coverage::record
    // applies that when the Owner's coverage is recorded.
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

/// What a Repo's Branches are compared against. When its target is its Upstream
/// but den can't see the Upstream's default branch, they are compared against
/// the fork's own (still facts worth having); `decide` sees the mismatch and
/// gives them no Next Step rather than judge them against the wrong branch.
fn comparison_branch(repo: &Repo, upstreams: &UpstreamTargets) -> Option<ComparisonBranch> {
    match comparison_for(repo, upstreams) {
        Comparison::Against(branch) => Some(branch),
        Comparison::Nothing => None,
        Comparison::UpstreamUnavailable { .. } => branches::own_default_branch(repo),
    }
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
        survey_request(&self.gql, request, Timestamp::now())
    }

    fn member_states(&self, request: &MemberStatesRequest) -> MemberStatesRead {
        member_states::read(&self.gql, request)
    }
}
