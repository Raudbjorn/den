//! The real GitHub source's Repo Survey (spec 0003, ADR-0005) over a scripted
//! GraphQL transport: one Repo, as GitHub spells it, every source recorded for
//! that Repo and none for its Owner, and no Owner-wide query sent.
//!
//! Thin adapter checks, like `tests/github_source.rs`; the real `gh` is
//! checked by hand (spec, Testing Decisions).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{Incompleteness, RepoId, Source, SourceKind, SourceState, UpstreamTargets},
    github::{
        graphql::{GraphQlError, GraphQlRequest, GraphQlResponse, TransportError},
        survey_repo, survey_repo_with,
    },
};
use serde_json::{Value, json};
use support::{
    branch_graphql::only_default_branches,
    build::{now, ts},
    fakes::{ScriptedGraphQl, data, operation},
};

const RESET_AT: &str = "2026-11-19T13:00:00Z";

fn page_info() -> Value {
    json!({ "hasNextPage": false, "endCursor": null })
}

fn pr_node(number: u64) -> Value {
    json!({
        "id": format!("PR_{number}"),
        "number": number,
        "title": format!("PR {number}"),
        "url": format!("https://github.com/Acme/App/pull/{number}"),
        "isDraft": true,
        "createdAt": "2026-11-01T00:00:00Z",
        "updatedAt": "2026-11-02T00:00:00Z",
        "author": { "login": "me" },
        "baseRefName": "main",
        "headRefName": format!("feature-{number}"),
        "headRefOid": "abc123",
        "headRepository": { "name": "App", "owner": { "login": "Acme" } },
        "mergeable": "MERGEABLE",
        "mergeStateStatus": "CLEAN",
        "reviewDecision": null,
        "isInMergeQueue": false
    })
}

fn issue_node(number: u64) -> Value {
    let none = json!({ "pageInfo": page_info(), "nodes": [] });
    json!({
        "number": number,
        "title": format!("Issue {number}"),
        "url": format!("https://github.com/Acme/App/issues/{number}"),
        "updatedAt": "2026-11-02T00:00:00Z",
        "labels": { "nodes": [] },
        "assignees": { "nodes": [] },
        "closingReferences": none,
        "manualLinks": none
    })
}

/// A details answer with empty checks, threads, requests, reviews and
/// comments for each PR asked about.
fn no_pr_details(request: &GraphQlRequest) -> Value {
    let empty = json!({ "pageInfo": page_info(), "nodes": [] });
    let mut answer = serde_json::Map::new();
    let asked = request
        .variables
        .as_object()
        .map_or(0, serde_json::Map::len);
    for i in 0..asked {
        answer.insert(
            format!("pr{i}"),
            json!({
                "headRefOid": "abc123",
                "reviewThreads": empty, "reviewRequests": empty, "latestOpinionatedReviews": empty, "latestReviews": empty,
                "reviews": empty, "comments": empty,
                "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] }
            }),
        );
    }
    Value::Object(answer)
}

/// GitHub, where the Repo asked for as `acme/app` is spelled `Acme/App`, or
/// is missing when `repository` is null.
fn github(repository: Value) -> ScriptedGraphQl {
    github_answering(data(json!({ "repository": repository })))
}

/// [`github`], answering `OneRepo` with `one_repo`, errors and all.
fn github_answering(one_repo: GraphQlResponse) -> ScriptedGraphQl {
    ScriptedGraphQl::new(move |request| {
        if let Some(answer) = only_default_branches(request) {
            return answer;
        }
        let answer: Result<GraphQlResponse, TransportError> = match operation(request).as_str() {
            "Viewer" => Ok(data(json!({
                "rateLimit": { "remaining": 4999, "resetAt": RESET_AT },
                "viewer": { "login": "me" }
            }))),
            "ViewerTeams" => Ok(data(json!({
                "viewer": { "organizations": { "pageInfo": { "hasNextPage": false }, "nodes": [] } }
            }))),
            "OneRepo" => Ok(one_repo.clone()),
            "RepoPullRequests" => Ok(data(json!({ "repository": { "pullRequests": {
                "pageInfo": page_info(), "nodes": [pr_node(4)]
            }}}))),
            "PullRequestDetails" => Ok(data(no_pr_details(request))),
            "RepoIssues" => Ok(data(json!({ "repository": { "issues": {
                "pageInfo": page_info(), "nodes": [issue_node(9)]
            }}}))),
            other => panic!("unexpected request {other}: {request:?}"),
        };
        answer
    })
}

fn acme_app() -> Value {
    json!({
        "name": "App",
        "owner": { "login": "Acme" },
        "url": "https://github.com/Acme/App",
        "isFork": false,
        "isArchived": false,
        "description": "The app",
        "pushedAt": "2026-11-18T09:00:00Z",
        "defaultBranchRef": { "name": "main" },
        "parent": null
    })
}

#[test]
fn a_repo_survey_reads_one_repo_as_github_spells_it_and_records_every_source_for_it() {
    let gql = github(acme_app());
    let observation = survey_repo(
        &gql,
        &RepoId::new("acme", "app"),
        &UpstreamTargets::default(),
        now(),
    )
    .expect("a survey");

    let spelled = RepoId::new("Acme", "App");
    let repo = observation
        .repo(&spelled)
        .expect("the Repo, as GitHub spells it");
    assert_eq!(repo.description.as_deref(), Some("The app"));
    assert_eq!(repo.pushed_at, Some(ts("2026-11-18T09:00:00Z")));
    assert_eq!(observation.pull_requests.len(), 1);
    assert_eq!(observation.pull_requests[0].repo, spelled);
    assert_eq!(observation.issues.len(), 1);
    assert_eq!(observation.issues[0].repo, spelled);
    assert!(
        observation.owners.is_empty(),
        "a Repo Survey covers no Owner"
    );
    assert_eq!(observation.observed_at, now());

    let state = |kind| {
        observation
            .coverage
            .get(&Source::Repo {
                repo: spelled.clone(),
                kind,
            })
            .cloned()
    };
    for kind in [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::PullRequestDetails,
        SourceKind::Issues,
        SourceKind::IssueDetails,
        SourceKind::Branches,
    ] {
        assert_eq!(state(kind), Some(SourceState::Complete), "{kind:?}");
    }
    // Runs are read over REST, which this GraphQL-only script has none of.
    assert_eq!(state(SourceKind::Runs), Some(SourceState::NotYetFetched));
    assert!(
        observation
            .coverage
            .entries()
            .iter()
            .all(|e| matches!(&e.source, Source::Viewer | Source::Repo { .. })),
        "nothing is recorded for the Owner: {:?}",
        observation.coverage
    );
    let operations: Vec<String> = gql.requests().iter().map(operation).collect();
    assert!(
        !operations
            .iter()
            .any(|o| ["OwnerRepos", "OwnerPullRequests", "OwnerIssueCounts"].contains(&o.as_str())),
        "{operations:?}"
    );
}

#[test]
fn a_repo_github_doesnt_have_fails_its_listing_and_asks_nothing_more() {
    let gql = github(Value::Null);
    let asked = RepoId::new("acme", "gone");
    let observation =
        survey_repo(&gql, &asked, &UpstreamTargets::default(), now()).expect("a survey");

    assert!(observation.repos.is_empty());
    let state = |kind| {
        observation
            .coverage
            .get(&Source::Repo {
                repo: asked.clone(),
                kind,
            })
            .cloned()
    };
    assert_eq!(
        state(SourceKind::Repos),
        Some(SourceState::Failed {
            error: "GitHub has no Repo acme/gone that den can see".to_owned()
        })
    );
    for kind in SourceKind::WORK_ITEMS {
        assert_eq!(state(kind), Some(SourceState::NotYetFetched), "{kind:?}");
    }
    let operations: Vec<String> = gql.requests().iter().map(operation).collect();
    assert_eq!(operations, ["Viewer", "ViewerTeams", "OneRepo"]);
}

/// Beyond the plan's Task 3 (#79 landed after it): a Repo Survey reads PR
/// bodies for screenshots only when its Repo is one of the screenshots Repos,
/// matched without case as everywhere else.
#[test]
fn a_repo_survey_asks_for_pr_bodies_only_for_a_screenshots_repo() {
    let bodies_asked = |screenshot_repos: &[RepoId]| {
        let gql = github(acme_app());
        survey_repo_with(
            &gql,
            &RepoId::new("acme", "app"),
            &UpstreamTargets::default(),
            screenshot_repos,
            now(),
        )
        .expect("a survey");
        let asked: Vec<Value> = gql
            .requests()
            .iter()
            .filter(|r| operation(r) == "RepoPullRequests")
            .map(|r| r.variables["bodies"].clone())
            .collect();
        asked
    };
    assert_eq!(bodies_asked(&[]), [json!(false)]);
    assert_eq!(
        bodies_asked(&[RepoId::new("other", "repo")]),
        [json!(false)]
    );
    assert_eq!(bodies_asked(&[RepoId::new("ACME", "App")]), [json!(true)]);
}

fn one_repo_error(kind: &str, message: &str) -> GraphQlError {
    GraphQlError {
        message: message.to_owned(),
        kind: Some(kind.to_owned()),
    }
}

/// A Repo listing read only partly (an error beside the data, which here
/// nulled its default branch) leaves no Work Item kind of the Repo complete,
/// as an Owner's incomplete Repo list does for the Owner's kinds: Branches
/// and Runs read nothing for want of the default branch, which is not
/// "there are none".
#[test]
fn a_partly_read_repo_leaves_none_of_its_work_items_complete() {
    let mut repository = acme_app();
    repository["defaultBranchRef"] = Value::Null;
    let gql = github_answering(GraphQlResponse {
        data: Some(json!({
            "rateLimit": { "remaining": 4000, "resetAt": RESET_AT },
            "repository": repository
        })),
        errors: vec![one_repo_error(
            "SERVICE_UNAVAILABLE",
            "defaultBranchRef timed out",
        )],
    });
    let observation = survey_repo(
        &gql,
        &RepoId::new("acme", "app"),
        &UpstreamTargets::default(),
        now(),
    )
    .expect("a survey");

    let spelled = RepoId::new("Acme", "App");
    for kind in SourceKind::WORK_ITEMS {
        let state = observation.coverage.get(&Source::Repo {
            repo: spelled.clone(),
            kind,
        });
        assert!(
            state.is_some_and(|s| !s.is_complete()),
            "{kind:?} is {state:?}"
        );
    }
}

/// A Repo GitHub answered null for because of the rate limit keeps the rate
/// limit and its reset time; it is not "no such Repo".
#[test]
fn a_rate_limited_repo_read_says_so_rather_than_that_the_repo_is_missing() {
    let gql = github_answering(GraphQlResponse {
        data: Some(json!({
            "rateLimit": { "remaining": 0, "resetAt": RESET_AT },
            "repository": null
        })),
        errors: vec![one_repo_error("RATE_LIMITED", "API rate limit exceeded")],
    });
    let asked = RepoId::new("acme", "app");
    let observation =
        survey_repo(&gql, &asked, &UpstreamTargets::default(), now()).expect("a survey");

    assert_eq!(
        observation.coverage.get(&Source::Repo {
            repo: asked,
            kind: SourceKind::Repos,
        }),
        Some(&SourceState::Partial(Incompleteness::RateLimited {
            resets_at: Some(ts(RESET_AT))
        }))
    );
}

/// When GitHub says why it answered null (SSO, permissions, not found), the
/// failed listing keeps GitHub's words beside den's.
#[test]
fn a_repo_github_refuses_keeps_githubs_reason() {
    let gql = github_answering(GraphQlResponse {
        data: Some(json!({
            "rateLimit": { "remaining": 4000, "resetAt": RESET_AT },
            "repository": null
        })),
        errors: vec![one_repo_error(
            "FORBIDDEN",
            "Resource protected by organization SAML enforcement.",
        )],
    });
    let asked = RepoId::new("acme", "app");
    let observation =
        survey_repo(&gql, &asked, &UpstreamTargets::default(), now()).expect("a survey");

    assert_eq!(
        observation.coverage.get(&Source::Repo {
            repo: asked,
            kind: SourceKind::Repos,
        }),
        Some(&SourceState::Failed {
            error: "GitHub has no Repo acme/app that den can see \
                    (Resource protected by organization SAML enforcement.)"
                .to_owned()
        })
    );
}
