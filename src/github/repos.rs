//! An Owner's own Repos, with fork / archived / default branch / Upstream.

use super::session::{Actor, Connection, Fetched, KindFetch, Pager, Session};
use crate::{
    domain::{CoverageEntry, Repo, RepoId, Source, SourceKind, SourceState},
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;

const QUERY: &str = include_str!("queries/repos.graphql");
const ONE_QUERY: &str = include_str!("queries/repo.graphql");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Data {
    repository_owner: Option<Owner>,
}

#[derive(Deserialize)]
struct Owner {
    repositories: Connection<RepoNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoNode {
    name: String,
    url: String,
    is_fork: bool,
    is_archived: bool,
    default_branch_ref: Option<Named>,
    parent: Option<Parent>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

/// The fork's Upstream: enough to compare against its default branch and to
/// target it (its URL), never its own parent.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Parent {
    name: String,
    owner: Actor,
    url: String,
    is_fork: bool,
    is_archived: bool,
    default_branch_ref: Option<Named>,
}

impl Parent {
    fn into_repo(self) -> Repo {
        Repo {
            id: RepoId::new(self.owner.login, self.name),
            url: self.url,
            is_fork: self.is_fork,
            is_archived: self.is_archived,
            default_branch: self.default_branch_ref.map(|r| r.name),
            parent: None,
            description: None,
            pushed_at: None,
        }
    }
}

pub fn fetch(session: &Session<'_>, owner: &str) -> Result<KindFetch<Repo>, SurveyFailure> {
    let mut out = KindFetch::default();
    let mut pager = Pager::new(None);
    while pager.may_continue() {
        let page =
            match session.run::<Data>(QUERY, json!({ "owner": owner, "after": pager.cursor }))? {
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
        out.items
            .extend(repos.nodes.into_iter().flatten().map(|node| Repo {
                id: RepoId::new(owner, node.name),
                url: node.url,
                is_fork: node.is_fork,
                is_archived: node.is_archived,
                default_branch: node.default_branch_ref.map(|r| r.name),
                parent: node.parent.map(|p| Box::new(p.into_repo())),
                description: None,
                pushed_at: None,
            }));
        if !pager.advance(repos.page_info.has_next_page, repos.page_info.end_cursor) {
            break;
        }
    }
    out.coverage.push(pager.finish(Source::Owner {
        owner: owner.to_owned(),
        kind: SourceKind::Repos,
    }));
    Ok(out)
}

#[derive(Deserialize)]
struct OneData {
    repository: Option<OneRepo>,
}

/// One Repo as a Repo Survey reads it (`repo.graphql`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OneRepo {
    name: String,
    owner: Actor,
    url: String,
    is_fork: bool,
    is_archived: bool,
    description: Option<String>,
    pushed_at: Option<Timestamp>,
    default_branch_ref: Option<Named>,
    parent: Option<Parent>,
}

/// Reads one Repo for a Repo Survey (spec 0003), identified as GitHub spells
/// its Owner and name, with its coverage as `Source::Repo { kind: Repos }`.
/// `None` when GitHub has no such Repo den can see, or the read was lost;
/// the coverage says which.
pub fn fetch_one(
    session: &Session<'_>,
    repo: &RepoId,
) -> Result<(Option<Repo>, CoverageEntry), SurveyFailure> {
    let asked = Source::Repo {
        repo: repo.clone(),
        kind: SourceKind::Repos,
    };
    let variables = json!({ "owner": repo.owner, "name": repo.name });
    let (node, state) = match session.run::<OneData>(ONE_QUERY, variables)? {
        Fetched::Data { data, state } => (data.repository, state),
        Fetched::Lost(state) => {
            return Ok((
                None,
                CoverageEntry {
                    source: asked,
                    state,
                },
            ));
        }
    };
    let Some(node) = node else {
        let state = SourceState::Failed {
            error: format!("GitHub has no Repo {repo} that den can see"),
        };
        return Ok((
            None,
            CoverageEntry {
                source: asked,
                state,
            },
        ));
    };
    let found = Repo {
        id: RepoId::new(node.owner.login, node.name),
        url: node.url,
        is_fork: node.is_fork,
        is_archived: node.is_archived,
        default_branch: node.default_branch_ref.map(|r| r.name),
        parent: node.parent.map(|p| Box::new(p.into_repo())),
        description: node.description,
        pushed_at: node.pushed_at,
    };
    let source = Source::Repo {
        repo: found.id.clone(),
        kind: SourceKind::Repos,
    };
    Ok((Some(found), CoverageEntry { source, state }))
}

pub fn no_such_owner(owner: &str) -> SourceState {
    SourceState::Failed {
        error: format!("GitHub has no user or organization called {owner}"),
    }
}
