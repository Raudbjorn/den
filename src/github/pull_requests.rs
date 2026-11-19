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
        CoverageEntry, RepoId, Source, SourceKind,
        facts::{Mergeability, PullRequest, ReviewDecision},
    },
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

const FACTS: &str = include_str!("queries/pull_request_facts.graphql");
const OWNER_QUERY: &str = include_str!("queries/pull_requests_owner.graphql");
const REPO_QUERY: &str = include_str!("queries/pull_requests_repo.graphql");

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
    review_decision: Option<String>,
    is_in_merge_queue: bool,
}

#[derive(Deserialize)]
struct HeadRepo {
    name: String,
    owner: Actor,
}

impl PrNode {
    /// The PR's node ID and its facts (details not read yet).
    fn into_fact(self, repo: &RepoId) -> (String, PullRequest) {
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
            review_decision: self.review_decision.as_deref().and_then(review_decision),
            in_merge_queue: self.is_in_merge_queue,
            details: None,
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

/// GitHub's `PullRequestReviewDecision`.
fn review_decision(decision: &str) -> Option<ReviewDecision> {
    match decision {
        "APPROVED" => Some(ReviewDecision::Approved),
        "CHANGES_REQUESTED" => Some(ReviewDecision::ChangesRequested),
        "REVIEW_REQUIRED" => Some(ReviewDecision::ReviewRequired),
        other => {
            tracing::warn!(decision = other, "unrecognised PR review decision; ignored");
            None
        }
    }
}

fn with_facts(query: &str) -> String {
    format!("{query}\n{FACTS}")
}

pub fn fetch(session: &Session<'_>, owner: &str) -> Result<KindFetch<PullRequest>, SurveyFailure> {
    let owner_query = with_facts(OWNER_QUERY);
    let mut out = KindFetch::default();
    let mut found_prs: Vec<(String, PullRequest)> = Vec::new();
    let mut pager = Pager::new(None);
    while pager.may_continue() {
        let page = match session.run::<OwnerData>(
            &owner_query,
            json!({ "owner": owner, "after": pager.cursor }),
        )? {
            Fetched::Data { data, state } => {
                pager.page_read(state);
                data
            }
            Fetched::Lost(state) => {
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
                    .map(|pr| pr.into_fact(&repo)),
            );
            if prs.page_info.has_next_page {
                fetch_rest_of_repo(
                    session,
                    &repo,
                    prs.page_info.end_cursor,
                    &mut found_prs,
                    &mut out.coverage,
                )?;
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
    details::fill(session, &mut found_prs, &mut out.coverage)?;
    out.items = found_prs.into_iter().map(|(_, pr)| pr).collect();
    Ok(out)
}

/// Reads one Repo's remaining PR pages; records that Repo's own coverage.
fn fetch_rest_of_repo(
    session: &Session<'_>,
    repo: &RepoId,
    cursor: Option<String>,
    found_prs: &mut Vec<(String, PullRequest)>,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<(), SurveyFailure> {
    let query = with_facts(REPO_QUERY);
    let mut pager = Pager::new(None);
    // The first page came with the Owner query.
    pager.page_read(crate::domain::SourceState::Complete);
    if !pager.advance(true, cursor) {
        coverage.push(pager.finish(repo_source(repo)));
        return Ok(());
    }
    while pager.may_continue() {
        let variables = json!({ "owner": repo.owner, "name": repo.name, "after": pager.cursor });
        let page = match session.run::<RepoData>(&query, variables)? {
            Fetched::Data { data, state } => {
                pager.page_read(state);
                data
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
        found_prs.extend(prs.nodes.into_iter().flatten().map(|pr| pr.into_fact(repo)));
        if !pager.advance(prs.page_info.has_next_page, prs.page_info.end_cursor) {
            break;
        }
    }
    coverage.push(pager.finish(repo_source(repo)));
    Ok(())
}

fn repo_source(repo: &RepoId) -> Source {
    Source::Repo {
        repo: repo.clone(),
        kind: SourceKind::PullRequests,
    }
}
