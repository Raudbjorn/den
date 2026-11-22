//! Open Pull Requests in an Owner's own Repos, paginated to completion: Repos
//! page by page with each Repo's first page of PRs, then further PR pages for
//! any Repo that has more. Each PR's checks and reviews then come from
//! [`details`], in batches.

mod details;

use super::{
    repos::no_such_owner,
    session::{Actor, Connection, Fetched, KindFetch, Pager, Session},
};
use crate::{
    domain::{
        CoverageEntry, RepoId, Source, SourceKind, SourceState,
        facts::{MergeState, Mergeability, PullRequest, ReviewDecision, Screenshots},
        ids::same_login,
    },
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

const FACTS: &str = include_str!("queries/pull_request_facts.graphql");
const OWNER_QUERY: &str = include_str!("queries/pull_requests_owner.graphql");
const REPO_QUERY: &str = include_str!("queries/pull_requests_repo.graphql");

/// Repos per OwnerPullRequests page. Each Repo carries up to 50 PRs with their
/// facts, so a page of many Repos can cost more than GitHub will answer.
pub const OWNER_REPOS_PER_PAGE: u32 = 25;
/// The fewest Repos per page den reads a lost Owner page again with, halving
/// from [`OWNER_REPOS_PER_PAGE`], before it gives the page up.
pub const MIN_OWNER_REPOS_PER_PAGE: u32 = 5;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnerData {
    repository_owner: Option<OwnerNode>,
}

#[derive(Deserialize)]
struct OwnerNode {
    repositories: Connection<RepoNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoNode {
    name: String,
    pull_requests: Connection<PrNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoData {
    repository: Option<RepoPrs>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoPrs {
    pull_requests: Connection<PrNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrNode {
    /// GitHub's node ID, which the details query needs.
    id: String,
    number: u64,
    title: String,
    url: String,
    is_draft: bool,
    created_at: Timestamp,
    updated_at: Timestamp,
    author: Option<Actor>,
    base_ref_name: String,
    head_ref_name: String,
    head_ref_oid: String,
    head_repository: Option<HeadRepo>,
    mergeable: String,
    merge_state_status: String,
    review_decision: Option<String>,
    is_in_merge_queue: bool,
    /// Present only when the query asked for bodies (`$bodies`, #79).
    #[serde(default)]
    body: Option<String>,
}

#[derive(Deserialize)]
struct HeadRepo {
    name: String,
    owner: Actor,
}

impl PrNode {
    /// The PR's node ID and its facts (details not read yet). `page_complete`
    /// is false when the page came with errors alongside data, which can null
    /// a nullable field such as the review decision. The body becomes the
    /// Screenshots state for the Repos in `screenshot_repos`, and is dropped.
    fn into_fact(
        self,
        repo: &RepoId,
        page_complete: bool,
        screenshot_repos: &[RepoId],
    ) -> (String, PullRequest) {
        let screenshots = match &self.body {
            Some(body) if screenshot_repos.iter().any(|r| r.same(repo)) => {
                Screenshots::from_body(body)
            }
            _ => Screenshots::NotRead,
        };
        let fact = PullRequest {
            repo: repo.clone(),
            number: self.number,
            title: self.title,
            url: self.url,
            author: self.author.map(|a| a.login),
            is_draft: self.is_draft,
            base_ref: self.base_ref_name,
            head_ref: self.head_ref_name,
            head_commit: self.head_ref_oid,
            head_repo: self
                .head_repository
                .map(|h| RepoId::new(h.owner.login, h.name)),
            created_at: self.created_at,
            updated_at: self.updated_at,
            mergeability: mergeability(&self.mergeable),
            merge_state: merge_state(&self.merge_state_status),
            review_decision: review_decision(self.review_decision.as_deref(), page_complete),
            in_merge_queue: self.is_in_merge_queue,
            details: None,
            screenshots,
        };
        (self.id, fact)
    }
}

/// GitHub's `MergeableState`. Anything den doesn't recognise counts as unknown.
fn mergeability(state: &str) -> Mergeability {
    match state {
        "MERGEABLE" => Mergeability::Mergeable,
        "CONFLICTING" => Mergeability::Conflicting,
        "UNKNOWN" => Mergeability::Unknown,
        other => {
            tracing::warn!(
                state = other,
                "unrecognised PR mergeability; treated as unknown"
            );
            Mergeability::Unknown
        }
    }
}

/// GitHub's `MergeStateStatus`. Anything den doesn't recognise counts as unknown.
fn merge_state(state: &str) -> MergeState {
    match state {
        "CLEAN" => MergeState::Clean,
        "HAS_HOOKS" => MergeState::HasHooks,
        "UNSTABLE" => MergeState::Unstable,
        "BLOCKED" => MergeState::Blocked,
        "BEHIND" => MergeState::Behind,
        "DIRTY" => MergeState::Dirty,
        "DRAFT" => MergeState::Draft,
        "UNKNOWN" => MergeState::Unknown,
        other => {
            tracing::warn!(
                state = other,
                "unrecognised PR merge state; treated as unknown"
            );
            MergeState::Unknown
        }
    }
}

/// GitHub's `PullRequestReviewDecision`. Null means no review is required,
/// unless the page carried errors: an error nulls the field it hits, so a null
/// there proves nothing.
fn review_decision(decision: Option<&str>, page_complete: bool) -> ReviewDecision {
    match decision {
        Some("APPROVED") => ReviewDecision::Approved,
        Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
        Some("REVIEW_REQUIRED") => ReviewDecision::ReviewRequired,
        Some(other) => {
            tracing::warn!(
                decision = other,
                "unrecognised PR review decision; treated as unknown"
            );
            ReviewDecision::Unknown
        }
        None if page_complete => ReviewDecision::NotRequired,
        None => ReviewDecision::Unknown,
    }
}

fn with_facts(query: &str) -> String {
    format!("{query}\n{FACTS}")
}

/// `viewer` is the GitHub viewer's login, for the Review Point facts (#75).
/// `screenshot_repos` are the Repos with `screenshots = true` (#79): PR bodies
/// are asked for only when this Owner has one, and decoded only for them.
pub fn fetch(
    session: &Session<'_>,
    owner: &str,
    viewer: &str,
    screenshot_repos: &[RepoId],
) -> Result<KindFetch<PullRequest>, SurveyFailure> {
    let owner_query = with_facts(OWNER_QUERY);
    let bodies = screenshot_repos
        .iter()
        .any(|repo| same_login(&repo.owner, owner));
    let mut out = KindFetch::default();
    let mut found_prs: Vec<(String, PullRequest)> = Vec::new();
    let mut pager = Pager::new(None);
    let mut batch = OWNER_REPOS_PER_PAGE;
    while pager.may_continue() {
        let (page, page_state) = match session.run::<OwnerData>(
            &owner_query,
            json!({ "owner": owner, "after": pager.cursor, "repos": batch, "bodies": bodies }),
        )? {
            Fetched::Data { data, state } => {
                pager.page_read(state.clone());
                (data, state)
            }
            Fetched::Lost(state) => {
                // Read the same page again with fewer Repos; the cursor has not moved.
                if let Some(smaller) = smaller_batch(batch, &state) {
                    tracing::warn!(
                        owner,
                        batch,
                        smaller,
                        error = %state.describe(),
                        "an Owner's PR page was lost; reading it again with fewer Repos"
                    );
                    batch = smaller;
                    continue;
                }
                pager.page_lost(state);
                break;
            }
        };
        let Some(found) = page.repository_owner else {
            pager.note(no_such_owner(owner));
            break;
        };
        let repos = found.repositories;
        for node in repos.nodes.into_iter().flatten() {
            let repo = RepoId::new(owner, node.name);
            let prs = node.pull_requests;
            found_prs.extend(
                prs.nodes
                    .into_iter()
                    .flatten()
                    .map(|pr| pr.into_fact(&repo, page_state.is_complete(), screenshot_repos)),
            );
            if prs.page_info.has_next_page {
                fetch_rest_of_repo(
                    session,
                    &repo,
                    RestOfRepo {
                        first_page: page_state.clone(),
                        cursor: prs.page_info.end_cursor,
                        pages: RepoPages {
                            screenshot_repos,
                            bodies,
                        },
                    },
                    &mut found_prs,
                    &mut out.coverage,
                )?;
            } else {
                // All of this Repo's PRs came on this page, so its own coverage
                // is this page's: a later Owner page lost does not touch it.
                out.coverage.push(CoverageEntry {
                    source: repo_source(&repo),
                    state: page_state.clone(),
                });
            }
        }
        if !pager.advance(repos.page_info.has_next_page, repos.page_info.end_cursor) {
            break;
        }
    }
    out.coverage.push(pager.finish(Source::Owner {
        owner: owner.to_owned(),
        kind: SourceKind::PullRequests,
    }));
    details::fill(session, &mut found_prs, &mut out.coverage, viewer)?;
    out.items = found_prs.into_iter().map(|(_, pr)| pr).collect();
    Ok(out)
}

/// The batch to read a lost Owner page again with, if any: half the Repos, down
/// to [`MIN_OWNER_REPOS_PER_PAGE`]. A page lost to the rate limit is not retried;
/// a smaller request would hit the same limit.
fn smaller_batch(batch: u32, lost: &SourceState) -> Option<u32> {
    if batch <= MIN_OWNER_REPOS_PER_PAGE || !matches!(lost, SourceState::Failed { .. }) {
        return None;
    }
    Some((batch / 2).max(MIN_OWNER_REPOS_PER_PAGE))
}

/// Every open PR of one Repo, with their checks and reviews, for a Repo
/// Survey (spec 0003): `pull_requests_repo.graphql` from its first page.
/// Coverage is the Repo's own; no Owner listing is read. `viewer` and
/// `screenshot_repos` are as for [`fetch`]; bodies are asked for only when
/// `repo` is one of `screenshot_repos`.
pub fn fetch_repo(
    session: &Session<'_>,
    repo: &RepoId,
    viewer: &str,
    screenshot_repos: &[RepoId],
) -> Result<KindFetch<PullRequest>, SurveyFailure> {
    let bodies = screenshot_repos.iter().any(|r| r.same(repo));
    let mut out = KindFetch::default();
    let mut found_prs: Vec<(String, PullRequest)> = Vec::new();
    let pages = RepoPages {
        screenshot_repos,
        bodies,
    };
    let entry = read_repo_pages(session, repo, Pager::new(None), pages, &mut found_prs)?;
    out.coverage.push(entry);
    details::fill(session, &mut found_prs, &mut out.coverage, viewer)?;
    out.items = found_prs.into_iter().map(|(_, pr)| pr).collect();
    Ok(out)
}

/// Where a Repo's further PR pages start, and how to read them.
struct RestOfRepo<'a> {
    /// The state of the Owner page its first PRs came on.
    first_page: SourceState,
    cursor: Option<String>,
    pages: RepoPages<'a>,
}

/// Reads one Repo's remaining PR pages; records that Repo's own coverage.
fn fetch_rest_of_repo(
    session: &Session<'_>,
    repo: &RepoId,
    rest: RestOfRepo<'_>,
    found_prs: &mut Vec<(String, PullRequest)>,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<(), SurveyFailure> {
    let RestOfRepo {
        first_page,
        cursor,
        pages,
    } = rest;
    let mut pager = Pager::new(None);
    // The first page came with the Owner query, so it carries that page's
    // state: an error there may have nulled some of this Repo's PRs.
    pager.page_read(first_page);
    if !pager.advance(true, cursor) {
        coverage.push(pager.finish(repo_source(repo)));
        return Ok(());
    }
    coverage.push(read_repo_pages(session, repo, pager, pages, found_prs)?);
    Ok(())
}

/// How a Repo's PR pages are read: whether bodies are asked for, and the
/// Repos whose bodies are decoded for screenshots (#79).
#[derive(Clone, Copy)]
struct RepoPages<'a> {
    screenshot_repos: &'a [RepoId],
    bodies: bool,
}

/// Reads one Repo's PR pages from `pager`'s cursor to the end (or den's
/// bound); returns that Repo's own coverage.
fn read_repo_pages(
    session: &Session<'_>,
    repo: &RepoId,
    mut pager: Pager,
    pages: RepoPages<'_>,
    found_prs: &mut Vec<(String, PullRequest)>,
) -> Result<CoverageEntry, SurveyFailure> {
    let RepoPages {
        screenshot_repos,
        bodies,
    } = pages;
    let query = with_facts(REPO_QUERY);
    while pager.may_continue() {
        let variables = json!({ "owner": repo.owner, "name": repo.name, "after": pager.cursor, "bodies": bodies });
        let (page, page_complete) = match session.run::<RepoData>(&query, variables)? {
            Fetched::Data { data, state } => {
                let complete = state.is_complete();
                pager.page_read(state);
                (data, complete)
            }
            Fetched::Lost(state) => {
                pager.page_lost(state);
                break;
            }
        };
        let Some(found) = page.repository else {
            pager.note(crate::domain::SourceState::Failed {
                error: format!("{repo} could no longer be read"),
            });
            break;
        };
        let prs = found.pull_requests;
        found_prs.extend(
            prs.nodes
                .into_iter()
                .flatten()
                .map(|pr| pr.into_fact(repo, page_complete, screenshot_repos)),
        );
        if !pager.advance(prs.page_info.has_next_page, prs.page_info.end_cursor) {
            break;
        }
    }
    Ok(pager.finish(repo_source(repo)))
}

fn repo_source(repo: &RepoId) -> Source {
    Source::Repo {
        repo: repo.clone(),
        kind: SourceKind::PullRequests,
    }
}
