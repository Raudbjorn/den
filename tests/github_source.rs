//! The real GitHub source over a scripted GraphQL transport (pagination,
//! partial and failed coverage), and the `gh` transport against fake `gh`
//! scripts (missing, logged out, timeouts, errors alongside data).
//!
//! These are thin adapter checks; the real `gh` is checked by hand (spec,
//! Testing Decisions).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{Incompleteness, Observation, Source, SourceKind, SourceState},
    github::{
        graphql::{GhCli, GraphQl, GraphQlError, GraphQlRequest, GraphQlResponse, TransportError},
        survey,
    },
    ports::SurveyFailure,
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, sync::Mutex, time::Duration};
use support::{
    branch_graphql::only_default_branches,
    build::{now, repo_id},
    fakes::{ScriptedGraphQl, data, operation},
};

const RESET_AT: &str = "2026-11-19T13:00:00Z";

fn viewer_answers(request: &GraphQlRequest) -> Option<Result<GraphQlResponse, TransportError>> {
    match operation(request).as_str() {
        "Viewer" => Some(Ok(data(json!({
            "rateLimit": { "remaining": 4999, "resetAt": RESET_AT },
            "viewer": { "login": "me" }
        })))),
        "ViewerTeams" => Some(Ok(data(json!({
            "viewer": { "organizations": {
                "pageInfo": { "hasNextPage": false },
                "nodes": [{ "login": "octo", "teams": {
                    "pageInfo": { "hasNextPage": false },
                    "nodes": [{ "slug": "core" }]
                }}]
            }}
        })))),
        // Every PR's checks and reviews: none, read completely.
        "PullRequestDetails" => Some(Ok(data(no_pr_details(request)))),
        _ => None,
    }
}

/// A details answer with empty checks, threads, requests and reviews for each PR asked about.
fn no_pr_details(request: &GraphQlRequest) -> Value {
    let empty = json!({ "pageInfo": page_info(None), "nodes": [] });
    let mut answer = serde_json::Map::new();
    for (i, _) in request
        .variables
        .as_object()
        .into_iter()
        .flatten()
        .enumerate()
    {
        answer.insert(
            format!("pr{i}"),
            json!({
                "reviewThreads": empty, "reviewRequests": empty, "latestOpinionatedReviews": empty,
                "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] }
            }),
        );
    }
    Value::Object(answer)
}

fn pr_node(number: u64, draft: bool) -> Value {
    json!({
        "id": format!("PR_{number}"),
        "number": number,
        "title": format!("PR {number}"),
        "url": format!("https://github.com/x/pull/{number}"),
        "isDraft": draft,
        "createdAt": "2026-11-01T00:00:00Z",
        "updatedAt": "2026-11-02T00:00:00Z",
        "author": { "login": "me" },
        "baseRefName": "main",
        "headRefName": format!("feature-{number}"),
        "headRefOid": "abc123",
        "headRepository": { "name": "fork", "owner": { "login": "someone" } },
        "mergeable": "MERGEABLE",
        "mergeStateStatus": "CLEAN",
        "reviewDecision": null,
        "isInMergeQueue": false
    })
}

fn page_info(next: Option<&str>) -> Value {
    json!({ "hasNextPage": next.is_some(), "endCursor": next })
}

fn repo_node(name: &str) -> Value {
    json!({
        "name": name,
        "url": format!("https://github.com/octo/{name}"),
        "isFork": name == "fork",
        "isArchived": false,
        "defaultBranchRef": { "name": "main" },
        "parent": if name == "fork" {
            json!({
                "name": "up", "owner": { "login": "upstream" },
                "url": "https://github.com/upstream/up", "isFork": false, "isArchived": false,
                "defaultBranchRef": { "name": "trunk" }
            })
        } else {
            Value::Null
        }
    })
}

fn after(request: &GraphQlRequest) -> Option<String> {
    request.variables["after"].as_str().map(str::to_owned)
}

/// Owner "octo": Repos over two pages; PRs over two Repo pages, where Repo
/// "busy" has three pages of PRs of its own.
fn paginated_octo(request: &GraphQlRequest) -> Result<GraphQlResponse, TransportError> {
    if let Some(answer) = viewer_answers(request).or_else(|| only_default_branches(request)) {
        return answer;
    }
    let page = after(request);
    match (operation(request).as_str(), page.as_deref()) {
        ("OwnerRepos", None) => Ok(data(json!({ "repositoryOwner": { "repositories": {
            "pageInfo": page_info(Some("r2")), "nodes": [repo_node("busy"), repo_node("fork")]
        }}}))),
        ("OwnerRepos", Some("r2")) => Ok(data(json!({ "repositoryOwner": { "repositories": {
            "pageInfo": page_info(None), "nodes": [repo_node("quiet")]
        }}}))),
        ("OwnerPullRequests", None) => Ok(data(json!({ "repositoryOwner": { "repositories": {
            "pageInfo": page_info(Some("p2")),
            "nodes": [
                { "name": "busy", "pullRequests": { "pageInfo": page_info(Some("b2")), "nodes": [pr_node(1, true), pr_node(2, false)] } },
                { "name": "fork", "pullRequests": { "pageInfo": page_info(None), "nodes": [pr_node(1, false)] } }
            ]
        }}}))),
        ("OwnerPullRequests", Some("p2")) => {
            Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(None),
                "nodes": [ { "name": "quiet", "pullRequests": { "pageInfo": page_info(None), "nodes": [] } } ]
            }}})))
        }
        ("RepoPullRequests", Some("b2")) => Ok(data(json!({ "repository": { "pullRequests": {
            "pageInfo": page_info(Some("b3")), "nodes": [pr_node(3, false)]
        }}}))),
        ("RepoPullRequests", Some("b3")) => Ok(data(json!({ "repository": { "pullRequests": {
            "pageInfo": page_info(None), "nodes": [pr_node(4, true)]
        }}}))),
        ("OwnerIssueCounts", _) => Ok(data(json!({ "repositoryOwner": { "repositories": {
            "pageInfo": page_info(None), "nodes": []
        }}}))),
        other => panic!("unexpected request {other:?}: {request:?}"),
    }
}

fn state(observation: &Observation, source: Source) -> SourceState {
    observation
        .coverage
        .get(&source)
        .cloned()
        .unwrap_or_else(|| panic!("no coverage for {source:?}: {:?}", observation.coverage))
}

fn owner_source(kind: SourceKind) -> Source {
    Source::Owner {
        owner: "octo".to_owned(),
        kind,
    }
}

#[test]
fn open_prs_are_paginated_to_completion_and_qualified_by_owner_and_repo() {
    let gql = ScriptedGraphQl::new(paginated_octo);
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");

    let mut prs: Vec<(String, u64, bool)> = observation
        .pull_requests
        .iter()
        .map(|p| (p.repo.to_string(), p.number, p.is_draft))
        .collect();
    prs.sort();
    assert_eq!(
        prs,
        [
            ("octo/busy".to_owned(), 1, true),
            ("octo/busy".to_owned(), 2, false),
            ("octo/busy".to_owned(), 3, false),
            ("octo/busy".to_owned(), 4, true),
            ("octo/fork".to_owned(), 1, false),
        ]
    );
    assert_eq!(observation.viewer.login, "me");
    assert_eq!(observation.viewer.teams.len(), 1);
    assert_eq!(observation.observed_at, now());
    let repos: Vec<String> = observation.repos.iter().map(|r| r.id.to_string()).collect();
    assert_eq!(repos, ["octo/busy", "octo/fork", "octo/quiet"]);
    let fork = observation.repo(&repo_id("octo/fork")).expect("fork");
    assert!(fork.is_fork);
    let upstream = fork.upstream().expect("the fork's Upstream");
    assert_eq!(upstream.id, repo_id("upstream/up"));
    assert_eq!(upstream.url, "https://github.com/upstream/up");
    assert_eq!(upstream.default_branch.as_deref(), Some("trunk"));

    for source in [
        Source::Viewer,
        owner_source(SourceKind::Repos),
        owner_source(SourceKind::PullRequests),
        Source::Repo {
            repo: repo_id("octo/busy"),
            kind: SourceKind::PullRequests,
        },
        owner_source(SourceKind::Branches),
    ] {
        assert_eq!(
            state(&observation, source.clone()),
            SourceState::Complete,
            "{source:?}"
        );
    }
    for kind in [SourceKind::Issues, SourceKind::Branches] {
        assert_eq!(
            state(&observation, owner_source(kind)),
            SourceState::Complete,
            "{kind:?}"
        );
    }
    // Runs are read over REST, which this GraphQL-only script has none of.
    assert_eq!(
        state(&observation, owner_source(SourceKind::Runs)),
        SourceState::NotYetFetched
    );
    // Variables travel as GraphQL variables, never spliced into query text.
    assert!(gql.requests().iter().all(|r| !r.query.contains("\"octo\"")));
}

/// A script where the Owner's PR query answers with `answer`; everything else is empty.
fn octo_prs_answer(
    answer: impl Fn() -> Result<GraphQlResponse, TransportError> + Send + Sync + 'static,
) -> ScriptedGraphQl {
    ScriptedGraphQl::new(move |request| {
        if let Some(viewer) = viewer_answers(request).or_else(|| only_default_branches(request)) {
            return viewer;
        }
        match operation(request).as_str() {
            "OwnerRepos" => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(None), "nodes": [repo_node("app")]
            }}}))),
            "OwnerPullRequests" => answer(),
            "OwnerIssueCounts" => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(None), "nodes": []
            }}}))),
            other => panic!("unexpected {other}"),
        }
    })
}

fn one_pr_page() -> Value {
    json!({ "repositoryOwner": { "repositories": {
        "pageInfo": page_info(None),
        "nodes": [ { "name": "app", "pullRequests": { "pageInfo": page_info(None), "nodes": [pr_node(1, false), null] } } ]
    }}})
}

#[test]
fn errors_alongside_data_are_partial_and_the_data_is_kept() {
    let gql = octo_prs_answer(|| {
        Ok(GraphQlResponse {
            data: Some(one_pr_page()),
            errors: vec![GraphQlError {
                message: "Resource not accessible by integration".to_owned(),
                kind: Some("FORBIDDEN".to_owned()),
            }],
        })
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(observation.pull_requests.len(), 1);
    assert_eq!(
        state(&observation, owner_source(SourceKind::PullRequests)),
        SourceState::Partial(Incompleteness::ErrorsAlongsideData {
            errors: vec!["Resource not accessible by integration".to_owned()]
        })
    );
}

#[test]
fn errors_without_data_fail_the_source_but_not_the_survey() {
    let gql = octo_prs_answer(|| {
        Ok(GraphQlResponse {
            data: None,
            errors: vec![GraphQlError {
                message: "Something went wrong".to_owned(),
                kind: None,
            }],
        })
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert!(observation.pull_requests.is_empty());
    assert_eq!(
        state(&observation, owner_source(SourceKind::PullRequests)),
        SourceState::Failed {
            error: "Something went wrong".to_owned()
        }
    );
    assert_eq!(
        state(&observation, owner_source(SourceKind::Repos)),
        SourceState::Complete
    );
}

#[test]
fn a_rate_limit_is_partial_with_the_reset_time_github_reported() {
    let gql = octo_prs_answer(|| {
        Ok(GraphQlResponse {
            data: Some(Value::Null),
            errors: vec![GraphQlError {
                message: "API rate limit exceeded for user ID 1.".to_owned(),
                kind: Some("RATE_LIMITED".to_owned()),
            }],
        })
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(
        state(&observation, owner_source(SourceKind::PullRequests)),
        SourceState::Partial(Incompleteness::RateLimited {
            resets_at: Some(RESET_AT.parse().expect("timestamp"))
        })
    );
}

/// A script where the Owner's PR list has two pages: the first carries data
/// and an error alongside it, the second answers with `second`.
fn octo_two_pr_pages(
    second: impl Fn() -> Result<GraphQlResponse, TransportError> + Send + Sync + 'static,
) -> ScriptedGraphQl {
    ScriptedGraphQl::new(move |request| {
        // The same answers as `octo_prs_answer` for everything but the PR list.
        if let Some(viewer) = viewer_answers(request).or_else(|| only_default_branches(request)) {
            return viewer;
        }
        match (operation(request).as_str(), after(request).as_deref()) {
            ("OwnerRepos", _) => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(None), "nodes": [repo_node("app")]
            }}}))),
            ("OwnerIssueCounts", _) => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(None), "nodes": []
            }}}))),
            ("OwnerPullRequests", None) => Ok(GraphQlResponse {
                data: Some(json!({ "repositoryOwner": { "repositories": {
                    "pageInfo": page_info(Some("p2")),
                    "nodes": [ { "name": "app", "pullRequests": { "pageInfo": page_info(None), "nodes": [pr_node(1, false)] } } ]
                }}})),
                errors: vec![GraphQlError {
                    message: "first page error".to_owned(),
                    kind: None,
                }],
            }),
            ("OwnerPullRequests", Some("p2")) => second(),
            other => panic!("unexpected {other:?}"),
        }
    })
}

#[test]
fn a_rate_limit_after_errors_alongside_data_keeps_the_rate_limit_and_reset_time() {
    let gql = octo_two_pr_pages(|| {
        Ok(GraphQlResponse {
            data: Some(Value::Null),
            errors: vec![GraphQlError {
                message: "API rate limit exceeded for user ID 1.".to_owned(),
                kind: Some("RATE_LIMITED".to_owned()),
            }],
        })
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(observation.pull_requests.len(), 1);
    // Pagination stopped because of the rate limit, so that is what coverage
    // reports (with when it resets), not the earlier page's error.
    assert_eq!(
        state(&observation, owner_source(SourceKind::PullRequests)),
        SourceState::Partial(Incompleteness::RateLimited {
            resets_at: Some(RESET_AT.parse().expect("timestamp"))
        })
    );
}

#[test]
fn errors_alongside_data_on_several_pages_are_all_kept() {
    let gql = octo_two_pr_pages(|| {
        Ok(GraphQlResponse {
            data: Some(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(None), "nodes": []
            }}})),
            errors: vec![GraphQlError {
                message: "second page error".to_owned(),
                kind: None,
            }],
        })
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(
        state(&observation, owner_source(SourceKind::PullRequests)),
        SourceState::Partial(Incompleteness::ErrorsAlongsideData {
            errors: vec![
                "first page error".to_owned(),
                "second page error".to_owned()
            ]
        })
    );
}

#[test]
fn a_page_lost_mid_pagination_makes_the_list_partial() {
    let gql = ScriptedGraphQl::new(|request| {
        if let Some(viewer) = viewer_answers(request) {
            return viewer;
        }
        match (operation(request).as_str(), after(request).as_deref()) {
            ("OwnerRepos", _) => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(None), "nodes": []
            }}}))),
            ("OwnerPullRequests", None) => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(Some("p2")),
                "nodes": [ { "name": "app", "pullRequests": { "pageInfo": page_info(None), "nodes": [pr_node(1, false)] } } ]
            }}}))),
            ("OwnerPullRequests", Some("p2")) => Err(TransportError::TimedOut {
                after: Duration::from_secs(30),
            }),
            ("OwnerIssueCounts", _) => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(None), "nodes": []
            }}}))),
            other => panic!("unexpected {other:?}"),
        }
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(observation.pull_requests.len(), 1, "the first page is kept");
    match state(&observation, owner_source(SourceKind::PullRequests)) {
        SourceState::Partial(Incompleteness::PaginationStopped { pages_read, reason }) => {
            assert_eq!(pages_read, 1);
            assert!(reason.contains("30s"), "{reason}");
        }
        other => panic!("expected partial, got {other:?}"),
    }
}

#[test]
fn an_unknown_owner_fails_its_sources_and_names_it() {
    let gql = ScriptedGraphQl::new(|request| {
        viewer_answers(request).unwrap_or_else(|| {
            Ok(GraphQlResponse {
                data: Some(json!({ "repositoryOwner": null })),
                errors: Vec::new(),
            })
        })
    });
    let observation = survey(&gql, &["nobody-here".to_owned()], now()).expect("survey");
    for kind in [SourceKind::Repos, SourceKind::PullRequests] {
        let state = state(
            &observation,
            Source::Owner {
                owner: "nobody-here".to_owned(),
                kind,
            },
        );
        assert!(
            matches!(&state, SourceState::Failed { error } if error.contains("nobody-here")),
            "{kind:?}: {state:?}"
        );
    }
}

#[test]
fn gh_missing_or_logged_out_stops_the_survey() {
    for (error, expected) in [
        (
            TransportError::GhNotInstalled,
            SurveyFailure::GhNotInstalled,
        ),
        (
            TransportError::NotLoggedIn {
                detail: "run gh auth login".to_owned(),
            },
            SurveyFailure::NotLoggedIn {
                detail: "run gh auth login".to_owned(),
            },
        ),
    ] {
        let failing = error.clone();
        let gql = ScriptedGraphQl::new(move |_| Err(failing.clone()));
        assert_eq!(survey(&gql, &["octo".to_owned()], now()), Err(expected));
    }
}

#[test]
fn lost_team_memberships_make_the_viewer_partial_not_the_survey_failed() {
    let gql = ScriptedGraphQl::new(|request| match operation(request).as_str() {
        "Viewer" => Ok(data(json!({ "viewer": { "login": "me" } }))),
        "ViewerTeams" => Err(TransportError::Failed {
            detail: "HTTP 502".to_owned(),
        }),
        _ => Ok(data(json!({ "repositoryOwner": { "repositories": {
            "pageInfo": page_info(None), "nodes": []
        }}}))),
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert!(matches!(
        state(&observation, Source::Viewer),
        SourceState::Partial(_)
    ));
}

// --- The `gh` transport against fake `gh` scripts ---------------------------

/// Serialises fake-`gh` tests: writing a script while another test thread forks
/// lets that child inherit the write handle, and exec then fails with "Text file
/// busy". (The real `gh` is never being written, so this is a test-only race.)
static FAKE_GH: Mutex<()> = Mutex::new(());

/// Writes a fake `gh` that runs `body` as a shell script, and runs `request()`
/// through it with `timeout`.
fn run_fake_gh(body: &str, timeout: Duration) -> Result<GraphQlResponse, TransportError> {
    let _serial = FAKE_GH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gh");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    GhCli::with_program(path, timeout).execute(&request())
}

fn request() -> GraphQlRequest {
    GraphQlRequest {
        query: "query Viewer { viewer { login } }".to_owned(),
        variables: json!({ "owner": "octo" }),
    }
}

const SHORT_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn gh_not_on_path_is_reported_as_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gh = GhCli::with_program(dir.path().join("no-such-gh"), SHORT_TIMEOUT);
    assert_eq!(gh.execute(&request()), Err(TransportError::GhNotInstalled));
}

#[test]
fn gh_logged_out_is_reported_with_what_to_run() {
    for script in [
        "echo 'To get started with GitHub CLI, please run:  gh auth login' >&2; exit 4",
        "echo '{\"message\":\"Bad credentials\",\"status\":\"401\"}'; echo 'gh: Bad credentials (HTTP 401)' >&2; exit 1",
    ] {
        let error = run_fake_gh(script, SHORT_TIMEOUT).expect_err(script);
        assert!(
            matches!(error, TransportError::NotLoggedIn { .. }),
            "{script}: {error:?}"
        );
    }
}

#[test]
fn a_body_with_errors_and_data_is_a_response_despite_gh_exiting_non_zero() {
    let script = r#"cat >/dev/null
echo '{"data":{"viewer":{"login":"me"},"repository":null},"errors":[{"type":"NOT_FOUND","message":"Could not resolve to a Repository"}]}'
echo "gh: Could not resolve to a Repository" >&2
exit 1"#;
    let response = run_fake_gh(script, SHORT_TIMEOUT).expect("a response");
    assert_eq!(
        response
            .data
            .as_ref()
            .and_then(|d| d.pointer("/viewer/login")),
        Some(&json!("me"))
    );
    assert_eq!(response.errors.len(), 1);
    assert_eq!(response.errors[0].kind.as_deref(), Some("NOT_FOUND"));
}

#[test]
fn the_request_goes_to_gh_api_graphql_on_stdin() {
    // Echo the arguments and the request body back as the response.
    let script = r#"body=$(cat)
printf '{"data":{"args":"%s","body":%s}}' "$*" "$body""#;
    let response = run_fake_gh(script, SHORT_TIMEOUT).expect("a response");
    let data = response.data.expect("data");
    assert_eq!(data["args"], json!("api graphql --input -"));
    assert_eq!(data["body"]["variables"]["owner"], json!("octo"));
    assert_eq!(data["body"]["query"], json!(request().query));
}

#[test]
fn a_hung_gh_is_stopped_at_the_timeout() {
    let timeout = Duration::from_millis(300);
    let started = std::time::Instant::now();
    assert_eq!(
        run_fake_gh("sleep 30", timeout),
        Err(TransportError::TimedOut { after: timeout })
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn other_gh_failures_are_values_with_context() {
    for (script, check) in [
        (
            "echo 'gh: API rate limit exceeded' >&2; exit 1",
            (|e: &TransportError| matches!(e, TransportError::RateLimited { .. }))
                as fn(&TransportError) -> bool,
        ),
        (
            "echo 'not json at all'; exit 0",
            |e| matches!(e, TransportError::Failed { detail } if detail.contains("no GraphQL response")),
        ),
        (
            "echo 'dial tcp: lookup api.github.com: no such host' >&2; exit 1",
            |e| matches!(e, TransportError::Failed { detail } if detail.contains("no such host") && detail.contains("exit 1")),
        ),
    ] {
        let error = run_fake_gh(script, SHORT_TIMEOUT).expect_err(script);
        assert!(check(&error), "{script}: {error:?}");
    }
}
