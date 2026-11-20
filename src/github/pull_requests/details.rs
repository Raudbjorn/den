//! Each open PR's checks, review threads, review requests and latest reviews,
//! read in batches of [`PRS_PER_QUERY`] PRs per query (one aliased `node(id:)`
//! per PR, since `isRequired` needs each PR's own ID).
//!
//! A batch that can't be read leaves its PRs' `details` as `None` and records
//! partial detail coverage on their Repos, separate from open-PR list coverage.
//! A batch with errors alongside data is kept,
//! and its Repos are marked partial too (an error can null out a nested field,
//! such as the check rollup, without anything else showing it, so a null
//! rollup in such a batch counts as checks not read, not as no checks). A PR
//! whose lists were not read in full (more entries than one query reads, or
//! unreadable entries) marks its Repo partial as well.

use crate::{
    domain::{
        CoverageEntry, Incompleteness, RepoId, Source, SourceKind, SourceState, TeamRef,
        facts::{
            Check, CheckOutcome, Listed, PrDetails, PullRequest, Review, ReviewVerdict, Reviewer,
        },
    },
    github::session::{Actor, Connection, Fetched, Session},
    ports::SurveyFailure,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// PRs per details query. Each PR asks for at most 300 nodes (100 checks,
/// 100 threads, 50 review requests, 50 reviews), so a batch stays far inside
/// GitHub's 500,000-node limit while keeping the number of `gh` calls low.
pub const PRS_PER_QUERY: usize = 25;

const SELECTION: &str = include_str!("../queries/pull_request_details.graphql");
const ID_PLACEHOLDER: &str = "$PR_ID";

/// Reads the details of every PR in `prs` (node ID, fact) into `fact.details`.
pub fn fill(
    session: &Session<'_>,
    prs: &mut [(String, PullRequest)],
    coverage: &mut Vec<CoverageEntry>,
) -> Result<(), SurveyFailure> {
    // Record clean details even for an empty list, so old carried detail ages expire.
    let completed: Vec<_> = coverage
        .iter()
        .filter_map(|entry| match &entry.source {
            Source::Repo {
                repo,
                kind: SourceKind::PullRequests,
            } => Some(CoverageEntry {
                source: repo_source(repo),
                state: SourceState::Complete,
            }),
            _ => None,
        })
        .collect();
    coverage.extend(completed);
    for batch in prs.chunks_mut(PRS_PER_QUERY) {
        fill_batch(session, batch, coverage)?;
    }
    Ok(())
}

fn fill_batch(
    session: &Session<'_>,
    batch: &mut [(String, PullRequest)],
    coverage: &mut Vec<CoverageEntry>,
) -> Result<(), SurveyFailure> {
    let variables: Map<String, Value> = batch
        .iter()
        .enumerate()
        .map(|(i, (node_id, _))| (format!("id{i}"), Value::String(node_id.clone())))
        .collect();
    let response =
        session.run::<Map<String, Value>>(&batch_query(batch.len()), Value::Object(variables))?;
    let (data, state) = match response {
        Fetched::Data { data, state } => (data, state),
        Fetched::Lost(state) => {
            let prs: Vec<&PullRequest> = batch.iter().map(|(_, pr)| pr).collect();
            record_lost(&prs, &state, coverage);
            return Ok(());
        }
    };
    let response_complete = state.is_complete();
    if !response_complete {
        for repo in repos_of(batch.iter().map(|(_, pr)| pr)).into_keys() {
            coverage.push(CoverageEntry {
                source: repo_source(&repo),
                state: state.clone(),
            });
        }
    }
    for (i, (_, pr)) in batch.iter_mut().enumerate() {
        let node = data.get(&format!("pr{i}")).cloned().unwrap_or(Value::Null);
        match serde_json::from_value::<Option<DetailsNode>>(node) {
            Ok(Some(details)) => pr.details = Some(details.into_details(response_complete)),
            Ok(None) => {
                let lost = SourceState::Failed {
                    error: "GitHub returned nothing for it".to_owned(),
                };
                record_lost(&[pr], &lost, coverage);
            }
            Err(e) => {
                let lost = SourceState::Failed {
                    error: format!("GitHub's response had an unexpected shape: {e}"),
                };
                record_lost(&[pr], &lost, coverage);
            }
        }
    }
    record_truncated(batch.iter().map(|(_, pr)| pr), coverage);
    Ok(())
}

/// Records, per Repo, the PRs whose details were read but not in full. The
/// per-PR flags reach the decision engine; this is what the header and sidebar
/// see.
fn record_truncated<'a>(
    prs: impl Iterator<Item = &'a PullRequest>,
    coverage: &mut Vec<CoverageEntry>,
) {
    let mut by_repo: BTreeMap<RepoId, Vec<String>> = BTreeMap::new();
    for pr in prs {
        let Some(details) = &pr.details else {
            continue;
        };
        let lists = truncated_lists(details);
        if lists.is_empty() {
            continue;
        }
        by_repo.entry(pr.repo.clone()).or_default().push(format!(
            "#{} ({})",
            pr.number,
            lists.join(", ")
        ));
    }
    for (repo, prs) in by_repo {
        let state = SourceState::Partial(Incompleteness::FollowUpTruncated {
            what: prs.join("; "),
        });
        tracing::warn!(repo = %repo, state = %state.describe(), "PR details not read in full");
        coverage.push(CoverageEntry {
            source: repo_source(&repo),
            state,
        });
    }
}

/// The names of a PR's lists that were not read in full.
fn truncated_lists(details: &PrDetails) -> Vec<&'static str> {
    [
        (!details.checks.complete, "checks"),
        (!details.threads_complete, "review threads"),
        (!details.review_requests.complete, "review requests"),
        (!details.latest_reviews.complete, "reviews"),
    ]
    .into_iter()
    .filter_map(|(truncated, name)| truncated.then_some(name))
    .collect()
}

/// The query for a batch of `len` PRs. Only digits are spliced into the text;
/// the IDs travel as variables.
fn batch_query(len: usize) -> String {
    let selection: String = SELECTION
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let declarations: Vec<String> = (0..len).map(|i| format!("$id{i}: ID!")).collect();
    let mut query = format!(
        "query PullRequestDetails({}) {{\n  rateLimit {{ remaining resetAt }}\n",
        declarations.join(", ")
    );
    for i in 0..len {
        query.push_str(&format!(
            "  pr{i}: node(id: $id{i}) {{\n    ... on PullRequest {{\n{}\n    }}\n  }}\n",
            selection.replace(ID_PLACEHOLDER, &format!("$id{i}"))
        ));
    }
    query.push_str("}\n");
    query
}

/// Records, per Repo, that these PRs' details could not be read. A rate limit
/// stays a rate limit (it carries the reset time); anything else is a lost
/// follow-up of a list that was itself read.
fn record_lost(prs: &[&PullRequest], state: &SourceState, coverage: &mut Vec<CoverageEntry>) {
    for (repo, numbers) in repos_of(prs.iter().copied()) {
        let state = match state {
            SourceState::Partial(Incompleteness::RateLimited { .. }) => state.clone(),
            other => SourceState::Partial(Incompleteness::FollowUpLost {
                what: format!("checks and reviews of {}", numbered(&numbers)),
                reason: other.describe(),
            }),
        };
        tracing::warn!(repo = %repo, state = %state.describe(), "PR details not read");
        coverage.push(CoverageEntry {
            source: repo_source(&repo),
            state,
        });
    }
}

fn repos_of<'a>(prs: impl Iterator<Item = &'a PullRequest>) -> BTreeMap<RepoId, Vec<u64>> {
    let mut by_repo: BTreeMap<RepoId, Vec<u64>> = BTreeMap::new();
    for pr in prs {
        by_repo.entry(pr.repo.clone()).or_default().push(pr.number);
    }
    by_repo
}

fn numbered(numbers: &[u64]) -> String {
    numbers
        .iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetailsNode {
    review_threads: Connection<ThreadNode>,
    review_requests: Connection<RequestNode>,
    latest_opinionated_reviews: Connection<ReviewNode>,
    commits: CommitList,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadNode {
    is_resolved: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestNode {
    requested_reviewer: Option<ReviewerNode>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum ReviewerNode {
    User {
        login: String,
    },
    Bot {
        login: String,
    },
    Mannequin {
        login: String,
    },
    Team {
        slug: String,
        organization: Actor,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct ReviewNode {
    state: String,
    author: Option<Actor>,
}

#[derive(Deserialize)]
struct CommitList {
    #[serde(default)]
    nodes: Vec<Option<CommitNode>>,
}

#[derive(Deserialize)]
struct CommitNode {
    commit: CommitFacts,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitFacts {
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    contexts: Connection<ContextNode>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum ContextNode {
    CheckRun {
        name: String,
        status: String,
        conclusion: Option<String>,
        #[serde(rename = "isRequired")]
        is_required: bool,
        #[serde(rename = "checkSuite")]
        check_suite: Option<SuiteNode>,
    },
    StatusContext {
        context: String,
        state: String,
        #[serde(rename = "isRequired")]
        is_required: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuiteNode {
    workflow_run: Option<WorkflowRunNode>,
}

#[derive(Deserialize)]
struct WorkflowRunNode {
    workflow: NamedNode,
}

#[derive(Deserialize)]
struct NamedNode {
    name: String,
}

impl DetailsNode {
    /// `response_complete` is false when the response carried errors.
    fn into_details(self, response_complete: bool) -> PrDetails {
        let threads = self.review_threads;
        let threads_complete =
            !threads.page_info.has_next_page && threads.nodes.iter().all(Option::is_some);
        let unresolved = threads.nodes.iter().flatten().filter(|t| !t.is_resolved);
        PrDetails {
            checks: checks(self.commits, response_complete),
            unresolved_threads: u32::try_from(unresolved.count()).unwrap_or(u32::MAX),
            threads_complete,
            review_requests: listed(self.review_requests, |node| {
                node.requested_reviewer
                    .and_then(ReviewerNode::into_reviewer)
            }),
            latest_reviews: listed(self.latest_opinionated_reviews, |node| {
                Some(Review {
                    author: node.author.map(|a| a.login),
                    verdict: verdict(&node.state),
                })
            }),
        }
    }
}

/// A connection as a [`Listed`]; nodes that are null or that `convert` can't
/// read make it incomplete.
fn listed<N, T>(connection: Connection<N>, convert: impl Fn(N) -> Option<T>) -> Listed<T> {
    let mut complete = !connection.page_info.has_next_page;
    let mut items = Vec::new();
    for node in connection.nodes {
        match node.and_then(&convert) {
            Some(item) => items.push(item),
            None => complete = false,
        }
    }
    Listed { items, complete }
}

/// The head commit's checks. No rollup (or no commit) means no checks were
/// ever reported, unless the response carried errors: an error nulls the field
/// it hits, so there a missing rollup means the checks were not read.
fn checks(commits: CommitList, response_complete: bool) -> Listed<Check> {
    let not_read = Listed {
        items: Vec::new(),
        complete: false,
    };
    match commits.nodes.into_iter().next_back() {
        None if response_complete => Listed::default(),
        None | Some(None) => not_read,
        Some(Some(node)) => match node.commit.status_check_rollup {
            None if response_complete => Listed::default(),
            None => not_read,
            Some(rollup) => listed(rollup.contexts, ContextNode::into_check),
        },
    }
}

impl ReviewerNode {
    fn into_reviewer(self) -> Option<Reviewer> {
        match self {
            Self::User { login } | Self::Bot { login } | Self::Mannequin { login } => {
                Some(Reviewer::User(login))
            }
            Self::Team { slug, organization } => Some(Reviewer::Team(TeamRef {
                org: organization.login,
                slug,
            })),
            Self::Other => None,
        }
    }
}

impl ContextNode {
    fn into_check(self) -> Option<Check> {
        match self {
            Self::CheckRun {
                name,
                status,
                conclusion,
                is_required,
                check_suite,
            } => {
                let workflow = check_suite
                    .and_then(|s| s.workflow_run)
                    .map(|r| r.workflow.name);
                Some(Check {
                    name: match workflow {
                        Some(workflow) => format!("{workflow} / {name}"),
                        None => name,
                    },
                    required: is_required,
                    outcome: check_run_outcome(&status, conclusion.as_deref()),
                })
            }
            Self::StatusContext {
                context,
                state,
                is_required,
            } => Some(Check {
                name: context,
                required: is_required,
                outcome: status_outcome(&state),
            }),
            Self::Other => None,
        }
    }
}

/// `CheckStatusState` and `CheckConclusionState`. A stale run never finishes
/// on its own, so it counts as failing (it needs a re-run), not as pending.
fn check_run_outcome(status: &str, conclusion: Option<&str>) -> CheckOutcome {
    match (status, conclusion) {
        ("COMPLETED", Some("SUCCESS" | "NEUTRAL" | "SKIPPED")) => CheckOutcome::Passing,
        (
            "COMPLETED",
            Some(
                "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
                | "STALE",
            ),
        ) => CheckOutcome::Failing,
        ("COMPLETED", Some(other)) => CheckOutcome::Unrecognised(other.to_owned()),
        ("COMPLETED", None) => CheckOutcome::Unrecognised("COMPLETED without a conclusion".into()),
        ("QUEUED" | "IN_PROGRESS" | "WAITING" | "PENDING" | "REQUESTED", _) => {
            CheckOutcome::Pending
        }
        (other, _) => CheckOutcome::Unrecognised(other.to_owned()),
    }
}

/// `StatusState` of a commit status.
fn status_outcome(state: &str) -> CheckOutcome {
    match state {
        "SUCCESS" => CheckOutcome::Passing,
        "FAILURE" | "ERROR" => CheckOutcome::Failing,
        "PENDING" | "EXPECTED" => CheckOutcome::Pending,
        other => CheckOutcome::Unrecognised(other.to_owned()),
    }
}

/// `PullRequestReviewState`.
fn verdict(state: &str) -> ReviewVerdict {
    match state {
        "APPROVED" => ReviewVerdict::Approved,
        "CHANGES_REQUESTED" => ReviewVerdict::ChangesRequested,
        _ => ReviewVerdict::Other,
    }
}

// Detail failures must never turn a fully enumerated open-PR list into a partial list.
fn repo_source(repo: &RepoId) -> Source {
    Source::Repo {
        repo: repo.clone(),
        kind: SourceKind::PullRequestDetails,
    }
}
