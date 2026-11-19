//! Open Issues in an Owner's own Repos, with the PRs linked to each.
//!
//! Two steps, to keep the GraphQL cost proportional to the Issues that exist:
//! a cheap count of open Issues per Repo (`issue_counts_owner.graphql`), then
//! each Repo that has any is read page by page (`issues_repo.graphql`). GitHub
//! prices a query by the page sizes it asks for, so nesting Issues and their
//! linked PRs under every Repo would cost for Repos that have none.
//!
//! Coverage: the count is recorded for the Owner, each Repo's Issues for that
//! Repo.

use super::{
    repos::no_such_owner,
    session::{Actor, Connection, Fetched, KindFetch, Pager, Session},
};
use crate::{
    domain::{
        CoverageEntry, Repo, RepoId, Source, SourceKind, SourceState,
        facts::{Issue, IssueLink, LinkKind, LinkedPrState},
    },
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

const FACTS: &str = include_str!("queries/issue_facts.graphql");
const COUNTS_QUERY: &str = include_str!("queries/issue_counts_owner.graphql");
const REPO_QUERY: &str = include_str!("queries/issues_repo.graphql");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CountsData {
    repository_owner: Option<CountsOwner>,
}

#[derive(Deserialize)]
struct CountsOwner {
    repositories: Connection<CountsRepo>,
}

#[derive(Deserialize)]
struct CountsRepo {
    name: String,
    issues: TotalCount,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TotalCount {
    total_count: u64,
}

#[derive(Deserialize)]
struct RepoData {
    repository: Option<RepoIssues>,
}

#[derive(Deserialize)]
struct RepoIssues {
    issues: Connection<IssueNode>,
}

/// A list read without pagination (`{ nodes }`).
#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct Nodes<T> {
    #[serde(default)]
    nodes: Vec<Option<T>>,
}

impl<T> Nodes<T> {
    fn into_items(self) -> impl Iterator<Item = T> {
        self.nodes.into_iter().flatten()
    }
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueNode {
    number: u64,
    title: String,
    url: String,
    updated_at: Timestamp,
    labels: Option<Nodes<Named>>,
    assignees: Option<Nodes<Actor>>,
    closing_references: Option<Connection<LinkedPrNode>>,
    manual_links: Option<Connection<LinkedPrNode>>,
}

#[derive(Deserialize)]
struct LinkedPrNode {
    number: u64,
    state: PrStateNode,
    repository: LinkedRepo,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PrStateNode {
    Open,
    Closed,
    Merged,
}

#[derive(Deserialize)]
struct LinkedRepo {
    name: String,
    owner: Actor,
}

impl IssueNode {
    fn into_fact(self, repo: &RepoId) -> Issue {
        let mut linked_prs = Vec::new();
        let mut linked_prs_complete = true;
        for (kind, links) in [
            (LinkKind::ClosingReference, self.closing_references),
            (LinkKind::ManualLink, self.manual_links),
        ] {
            let Some(links) = links else {
                // GitHub left the list out (an error came instead).
                linked_prs_complete = false;
                continue;
            };
            if links.page_info.has_next_page || links.nodes.iter().any(Option::is_none) {
                linked_prs_complete = false;
            }
            linked_prs.extend(links.nodes.into_iter().flatten().map(|pr| IssueLink {
                kind,
                repo: RepoId::new(pr.repository.owner.login, pr.repository.name),
                number: pr.number,
                state: match pr.state {
                    PrStateNode::Open => LinkedPrState::Open,
                    PrStateNode::Closed => LinkedPrState::Closed,
                    PrStateNode::Merged => LinkedPrState::Merged,
                },
            }));
        }
        Issue {
            repo: repo.clone(),
            number: self.number,
            title: self.title,
            url: self.url,
            labels: self
                .labels
                .map(|l| l.into_items().map(|n| n.name).collect())
                .unwrap_or_default(),
            assignees: self
                .assignees
                .map(|a| a.into_items().map(|n| n.login).collect())
                .unwrap_or_default(),
            updated_at: self.updated_at,
            linked_prs,
            linked_prs_complete,
        }
    }
}

pub fn fetch(
    session: &Session<'_>,
    owner: &str,
    _repos: &[Repo],
) -> Result<KindFetch<Issue>, SurveyFailure> {
    let mut out = KindFetch::default();
    let (with_issues, counts_coverage) = repos_with_open_issues(session, owner)?;
    out.coverage.push(counts_coverage);
    for repo in with_issues {
        fetch_repo(session, &repo, &mut out)?;
    }
    Ok(out)
}

/// The Owner's Repos that have open Issues, and how completely that was read.
fn repos_with_open_issues(
    session: &Session<'_>,
    owner: &str,
) -> Result<(Vec<RepoId>, CoverageEntry), SurveyFailure> {
    let mut repos = Vec::new();
    let mut pager = Pager::new(None);
    while pager.may_continue() {
        let page = match session.run::<CountsData>(
            COUNTS_QUERY,
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
        let page = found.repositories;
        repos.extend(
            page.nodes
                .into_iter()
                .flatten()
                .filter(|r| r.issues.total_count > 0)
                .map(|r| RepoId::new(owner, r.name)),
        );
        if !pager.advance(page.page_info.has_next_page, page.page_info.end_cursor) {
            break;
        }
    }
    let coverage = pager.finish(Source::Owner {
        owner: owner.to_owned(),
        kind: SourceKind::Issues,
    });
    Ok((repos, coverage))
}

/// Reads every page of one Repo's open Issues; records that Repo's coverage.
fn fetch_repo(
    session: &Session<'_>,
    repo: &RepoId,
    out: &mut KindFetch<Issue>,
) -> Result<(), SurveyFailure> {
    let query = format!("{REPO_QUERY}\n{FACTS}");
    let mut pager = Pager::new(None);
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
            pager.note(SourceState::Failed {
                error: format!("{repo} could no longer be read"),
            });
            break;
        };
        let issues = found.issues;
        out.items.extend(
            issues
                .nodes
                .into_iter()
                .flatten()
                .map(|issue| issue.into_fact(repo)),
        );
        if !pager.advance(issues.page_info.has_next_page, issues.page_info.end_cursor) {
            break;
        }
    }
    out.coverage.push(pager.finish(Source::Repo {
        repo: repo.clone(),
        kind: SourceKind::Issues,
    }));
    Ok(())
}
