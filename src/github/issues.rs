//! Open Issues in an Owner's own Repos, with the PRs linked to each.
//!
//! Two steps, to keep the GraphQL cost proportional to the Issues that exist:
//! a cheap count of open Issues per Repo (`issue_counts_owner.graphql`), then
//! each Repo that has any is read page by page (`issues_repo.graphql`). GitHub
//! prices a query by the page sizes it asks for, so nesting Issues and their
//! linked PRs under every Repo would cost for Repos that have none.
//!
//! Coverage: the count is recorded for the Owner, each Repo's Issues for that
//! Repo. When GitHub is unreachable for several Repos in a row, the rest are not
//! asked about and are recorded as not read, so an outage costs a bounded number
//! of `gh` timeouts rather than one per Repo.

use super::{
    repos::no_such_owner,
    session::{Actor, Connection, Fetched, KindFetch, Pager, Session},
};
use crate::{
    domain::{
        CoverageEntry, Incompleteness, Repo, RepoId, Source, SourceKind, SourceState,
        facts::{Issue, IssueLink, LinkKind, LinkedPrState},
    },
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

/// Repos in a row whose Issues could not be reached at all before den takes
/// GitHub to be unreachable and stops asking. Each try can cost a whole
/// `gh` timeout, so without a stop an outage costs one per Repo with Issues.
const UNREACHABLE_REPOS_BEFORE_STOPPING: u32 = 2;

/// Pages of linked PRs den reads per Issue and link kind (`issue_facts.graphql`
/// asks for one page and never follows its cursor).
const LINK_PAGES_READ: u32 = 1;

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

/// A list's items, and whether every one was read: false when GitHub left the
/// list out or some entries were null (both come next to an error).
fn read_list<T, U>(list: Option<Nodes<T>>, item: impl Fn(T) -> U) -> (Vec<U>, bool) {
    let Some(list) = list else {
        return (Vec::new(), false);
    };
    let complete = list.nodes.iter().all(Option::is_some);
    (
        list.nodes.into_iter().flatten().map(item).collect(),
        complete,
    )
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
    /// The Issue fact, and whether GitHub had more linked PRs than den asked for
    /// (den stopped reading them; no error says so).
    fn into_fact(self, repo: &RepoId) -> (Issue, bool) {
        let mut linked_prs = Vec::new();
        let mut linked_prs_complete = true;
        let mut links_cut_short = false;
        for (kind, links) in [
            (LinkKind::ClosingReference, self.closing_references),
            (LinkKind::ManualLink, self.manual_links),
        ] {
            let Some(links) = links else {
                // GitHub left the list out (an error came instead).
                linked_prs_complete = false;
                continue;
            };
            if links.page_info.has_next_page {
                links_cut_short = true;
            }
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
        let (labels, labels_complete) = read_list(self.labels, |n| n.name);
        let (assignees, assignees_complete) = read_list(self.assignees, |n| n.login);
        let issue = Issue {
            repo: repo.clone(),
            number: self.number,
            title: self.title,
            url: self.url,
            labels,
            labels_complete,
            assignees,
            assignees_complete,
            updated_at: self.updated_at,
            linked_prs,
            linked_prs_complete,
        };
        (issue, links_cut_short)
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
    let mut unreachable_in_a_row = 0;
    let mut repos = with_issues.into_iter();
    while let Some(repo) = repos.next() {
        match fetch_repo(session, &repo, &mut out)? {
            RepoRead::Reached => unreachable_in_a_row = 0,
            RepoRead::Unreachable(why) => {
                unreachable_in_a_row += 1;
                if unreachable_in_a_row >= UNREACHABLE_REPOS_BEFORE_STOPPING {
                    let skipped = skipped_state(&why);
                    out.coverage.extend(repos.map(|repo| CoverageEntry {
                        source: Source::Repo {
                            repo,
                            kind: SourceKind::Issues,
                        },
                        state: skipped.clone(),
                    }));
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Every open Issue of one Repo, for a Repo Survey (spec 0003). Coverage is
/// the Repo's own: no Owner count is read.
pub fn fetch_one(session: &Session<'_>, repo: &RepoId) -> Result<KindFetch<Issue>, SurveyFailure> {
    let mut out = KindFetch::default();
    fetch_repo(session, repo, &mut out)?;
    Ok(out)
}

/// Whether GitHub answered for a Repo at all.
enum RepoRead {
    Reached,
    /// The first page was lost; the state says why.
    Unreachable(SourceState),
}

/// The coverage of a Repo den did not ask about because GitHub was unreachable:
/// rate-limited stays rate-limited (with its reset time), anything else fails.
fn skipped_state(why: &SourceState) -> SourceState {
    match why {
        SourceState::Partial(Incompleteness::RateLimited { .. }) => why.clone(),
        other => SourceState::Failed {
            error: format!(
                "not read: GitHub was unreachable for {UNREACHABLE_REPOS_BEFORE_STOPPING} Repos in a row ({})",
                other.describe()
            ),
        },
    }
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
) -> Result<RepoRead, SurveyFailure> {
    let query = format!("{REPO_QUERY}\n{FACTS}");
    let mut pager = Pager::new(None);
    let mut read = RepoRead::Reached;
    let mut first_page = true;
    let mut links_cut_short = Vec::new();
    while pager.may_continue() {
        let variables = json!({ "owner": repo.owner, "name": repo.name, "after": pager.cursor });
        let page = match session.run::<RepoData>(&query, variables)? {
            Fetched::Data { data, state } => {
                pager.page_read(state);
                data
            }
            Fetched::Lost(state) => {
                if first_page {
                    read = RepoRead::Unreachable(state.clone());
                }
                pager.page_lost(state);
                break;
            }
        };
        first_page = false;
        let Some(found) = page.repository else {
            pager.note(SourceState::Failed {
                error: format!("{repo} could no longer be read"),
            });
            break;
        };
        let issues = found.issues;
        for (issue, cut_short) in issues
            .nodes
            .into_iter()
            .flatten()
            .map(|n| n.into_fact(repo))
        {
            if cut_short {
                links_cut_short.push(format!("#{}", issue.number));
            }
            out.items.push(issue);
        }
        if !pager.advance(issues.page_info.has_next_page, issues.page_info.end_cursor) {
            break;
        }
    }
    let enumeration = pager.finish(Source::Repo {
        repo: repo.clone(),
        kind: SourceKind::Issues,
    });
    // Truncated nested links must not keep absent (closed) Issues in the cache.
    let details = if links_cut_short.is_empty() {
        enumeration.state.clone()
    } else {
        SourceState::Partial(Incompleteness::PaginationStopped {
            pages_read: LINK_PAGES_READ,
            reason: format!(
                "more linked PRs than den reads on {}",
                links_cut_short.join(", ")
            ),
        })
    };
    out.coverage.push(enumeration);
    out.coverage.push(CoverageEntry {
        source: Source::Repo {
            repo: repo.clone(),
            kind: SourceKind::IssueDetails,
        },
        state: details,
    });
    Ok(read)
}
