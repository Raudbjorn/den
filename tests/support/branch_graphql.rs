//! A scripted GitHub for the Branch fetcher's three batched queries
//! (`BranchRefs`, `BranchCandidates`, `BranchComparisons`): Repos with
//! Branches, answered per alias like GitHub does, with offset cursors.

use super::fakes::{data, operation};
use den::github::graphql::{GraphQlRequest, GraphQlResponse, TransportError};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The `refs` page size the fetcher asks for (read from the query in tests that care).
const PAGE: usize = 100;

#[derive(Clone, Debug)]
pub struct FakeBranch {
    pub name: String,
    pub committed: String,
    pub author: Option<String>,
    pub headline: String,
    /// PRs that ever used this Branch as head.
    pub prs: u64,
    /// Commits ahead of whatever it is compared with.
    pub ahead: u32,
}

impl FakeBranch {
    pub fn new(name: &str, committed: &str) -> Self {
        Self {
            name: name.to_owned(),
            committed: committed.to_owned(),
            author: Some("me".to_owned()),
            headline: format!("work on {name}"),
            prs: 0,
            ahead: 1,
        }
    }

    pub fn prs(mut self, prs: u64) -> Self {
        self.prs = prs;
        self
    }

    pub fn ahead(mut self, ahead: u32) -> Self {
        self.ahead = ahead;
        self
    }

    pub fn unmatched(mut self) -> Self {
        self.author = None;
        self
    }
}

#[derive(Clone, Debug)]
pub struct FakeRepo {
    pub created_at: String,
    /// In GitHub's (alphabetical) order.
    pub branches: Vec<FakeBranch>,
}

/// Repos keyed by `owner/name`.
#[derive(Clone, Debug, Default)]
pub struct FakeBranches {
    pub repos: BTreeMap<String, FakeRepo>,
}

impl FakeBranches {
    pub fn repo(mut self, full_name: &str, created_at: &str, branches: Vec<FakeBranch>) -> Self {
        let mut branches = branches;
        branches.sort_by(|a, b| a.name.cmp(&b.name));
        self.repos.insert(
            full_name.to_owned(),
            FakeRepo {
                created_at: created_at.to_owned(),
                branches,
            },
        );
        self
    }

    /// Answers the three Branch queries; `None` for any other request.
    pub fn answer(
        &self,
        request: &GraphQlRequest,
    ) -> Option<Result<GraphQlResponse, TransportError>> {
        let op = operation(request);
        let prefix = match op.as_str() {
            "BranchRefs" => "r",
            "BranchCandidates" | "BranchComparisons" => "c",
            _ => return None,
        };
        let vars = &request.variables;
        let mut out = Map::new();
        for i in 0.. {
            let Some(owner) = vars.get(format!("owner{i}")).and_then(Value::as_str) else {
                break;
            };
            let name = vars[format!("name{i}")].as_str().unwrap_or_default();
            let repo = self.repos.get(&format!("{owner}/{name}"));
            let value = match (op.as_str(), repo) {
                (_, None) => Value::Null,
                ("BranchRefs", Some(repo)) => refs_page(repo, request, i),
                ("BranchCandidates", Some(repo)) => {
                    let wanted = vars[format!("ref{i}")].as_str().unwrap_or_default();
                    let branch = repo
                        .branches
                        .iter()
                        .find(|b| Some(b.name.as_str()) == wanted.strip_prefix("refs/heads/"));
                    json!({ "ref": branch.map(|b| json!({
                        "associatedPullRequests": { "totalCount": b.prs },
                        "target": {
                            "committedDate": b.committed,
                            "messageHeadline": b.headline,
                            "author": { "user": b.author.as_ref().map(|l| json!({ "login": l })) }
                        }
                    })) })
                }
                (_, Some(_)) => {
                    // BranchComparisons: the head is `refs/heads/x` or `owner:x`.
                    let head = vars[format!("head{i}")].as_str().unwrap_or_default();
                    let ahead = self.find_head(owner, name, head).map(|b| b.ahead);
                    json!({ "ref": { "compare": ahead.map(|a| json!({ "aheadBy": a })) } })
                }
            };
            out.insert(format!("{prefix}{i}"), value);
        }
        Some(Ok(data(Value::Object(out))))
    }

    fn find_head(&self, owner: &str, name: &str, head: &str) -> Option<&FakeBranch> {
        let (repo, branch) = match head.strip_prefix("refs/heads/") {
            Some(branch) => (format!("{owner}/{name}"), branch),
            None => {
                let (head_owner, branch) = head.split_once(':')?;
                // Cross-Repo heads: the fork of `name` owned by `head_owner`.
                (format!("{head_owner}/{name}"), branch)
            }
        };
        self.repos
            .get(&repo)
            .and_then(|r| r.branches.iter().find(|b| b.name == branch))
            .or_else(|| {
                // A fork named differently: search every Repo of that owner.
                let head_owner = head.split_once(':')?.0;
                self.repos
                    .iter()
                    .filter(|(k, _)| k.starts_with(&format!("{head_owner}/")))
                    .find_map(|(_, r)| r.branches.iter().find(|b| b.name == branch))
            })
    }
}

/// One page of `refs` for alias `i`, forwards or backwards as the query asks.
fn refs_page(repo: &FakeRepo, request: &GraphQlRequest, i: usize) -> Value {
    let backward = alias_reads_backward(&request.query, i);
    let total = repo.branches.len();
    let cursor = request.variables[format!("cursor{i}")]
        .as_str()
        .and_then(|c| c.parse::<usize>().ok());
    let (start, end) = if backward {
        let end = cursor.unwrap_or(total);
        (end.saturating_sub(PAGE), end)
    } else {
        let start = cursor.unwrap_or(0);
        (start, (start + PAGE).min(total))
    };
    let nodes: Vec<Value> = repo.branches[start..end]
        .iter()
        .map(|b| json!({ "name": b.name, "target": { "committedDate": b.committed } }))
        .collect();
    json!({
        "createdAt": repo.created_at,
        "refs": {
            "totalCount": total,
            "pageInfo": {
                "hasNextPage": end < total,
                "hasPreviousPage": start > 0,
                "startCursor": start.to_string(),
                "endCursor": end.to_string()
            },
            "nodes": nodes
        }
    })
}

/// Whether alias `r{i}` asks for `last:` (backwards) rather than `first:`.
fn alias_reads_backward(query: &str, i: usize) -> bool {
    let marker = format!("r{i}: repository");
    query
        .find(&marker)
        .and_then(|at| {
            let rest = &query[at..];
            let refs = rest.find("refs(")?;
            let close = rest[refs..].find(')')?;
            Some(rest[refs..refs + close].contains("last:"))
        })
        .unwrap_or(false)
}

/// Every Repo of these names has only its default branch `main`.
pub fn only_default_branches(
    request: &GraphQlRequest,
) -> Option<Result<GraphQlResponse, TransportError>> {
    if operation(request) != "BranchRefs" {
        return None;
    }
    let mut out = Map::new();
    for i in 0.. {
        if request.variables.get(format!("owner{i}")).is_none() {
            break;
        }
        out.insert(
            format!("r{i}"),
            json!({
                "createdAt": "2026-01-01T00:00:00Z",
                "refs": {
                    "totalCount": 1,
                    "pageInfo": { "hasNextPage": false, "hasPreviousPage": false, "startCursor": "0", "endCursor": "1" },
                    "nodes": [{ "name": "main", "target": { "committedDate": "2026-11-01T00:00:00Z" } }]
                }
            }),
        );
    }
    Some(Ok(data(Value::Object(out))))
}
