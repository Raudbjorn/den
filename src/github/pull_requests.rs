//! Open Pull Requests in an Owner's own Repos, paginated to completion: Repos
//! page by page with each Repo's first page of PRs, then further PR pages for
//! any Repo that has more. Owned by #4 when it adds the facts its rules need.

use super::{
    repos::no_such_owner,
    session::{Actor, Connection, Fetched, KindFetch, Pager, Session},
};
use crate::{
    domain::{RepoId, Source, SourceKind, facts::PullRequest},
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
}

#[derive(Deserialize)]
struct HeadRepo {
    name: String,
    owner: Actor,
}

impl PrNode {
    fn into_fact(self, repo: &RepoId) -> PullRequest {
        PullRequest {
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
        }
    }
}

fn with_facts(query: &str) -> String {
    format!("{query}\n{FACTS}")
}

pub fn fetch(session: &Session<'_>, owner: &str) -> Result<KindFetch<PullRequest>, SurveyFailure> {
    let owner_query = with_facts(OWNER_QUERY);
    let mut out = KindFetch::default();
    let mut pager = Pager::new(None);
    while pager.may_continue() {
        let (page, page_state) = match session.run::<OwnerData>(
            &owner_query,
            json!({ "owner": owner, "after": pager.cursor }),
        )? {
            Fetched::Data { data, state } => {
                pager.page_read(state.clone());
                (data, state)
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
            out.items.extend(
                prs.nodes
                    .into_iter()
                    .flatten()
                    .map(|pr| pr.into_fact(&repo)),
            );
            if prs.page_info.has_next_page {
                fetch_rest_of_repo(
                    session,
                    &repo,
                    page_state.clone(),
                    prs.page_info.end_cursor,
                    &mut out,
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
    Ok(out)
}

/// Reads one Repo's remaining PR pages; records that Repo's own coverage.
fn fetch_rest_of_repo(
    session: &Session<'_>,
    repo: &RepoId,
    first_page: crate::domain::SourceState,
    cursor: Option<String>,
    out: &mut KindFetch<PullRequest>,
) -> Result<(), SurveyFailure> {
    let query = with_facts(REPO_QUERY);
    let mut pager = Pager::new(None);
    // The first page came with the Owner query, so it carries that page's
    // state: an error there may have nulled some of this Repo's PRs.
    pager.page_read(first_page);
    if !pager.advance(true, cursor) {
        out.coverage.push(pager.finish(repo_source(repo)));
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
        out.items
            .extend(prs.nodes.into_iter().flatten().map(|pr| pr.into_fact(repo)));
        if !pager.advance(prs.page_info.has_next_page, prs.page_info.end_cursor) {
            break;
        }
    }
    out.coverage.push(pager.finish(repo_source(repo)));
    Ok(())
}

fn repo_source(repo: &RepoId) -> Source {
    Source::Repo {
        repo: repo.clone(),
        kind: SourceKind::PullRequests,
    }
}
