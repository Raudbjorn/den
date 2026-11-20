//! The real GitHub source's PR facts over a scripted GraphQL transport:
//! mergeability, merge state, review decision and merge queue from the PR
//! list (a null decision on a page with errors is unknown); checks
//! (required / advisory, passing / failing / pending), review threads, review
//! requests (users and teams) and latest reviews from the batched details
//! query; and what coverage says when that follow-up query is lost or not
//! read in full.
//!
//! Thin adapter checks; the real `gh` is checked by hand (spec, Testing Decisions).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{
        Incompleteness, Observation, Source, SourceKind, SourceState, TeamRef,
        facts::{
            ActorKind, Check, CheckOutcome, MergeState, Mergeability, PullRequest, Review,
            ReviewDecision, ReviewVerdict, Reviewer,
        },
    },
    github::{
        graphql::{GraphQlError, GraphQlRequest, GraphQlResponse, TransportError},
        survey,
    },
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use support::{
    branch_graphql::only_default_branches,
    build::{now, repo_id},
    fakes::{ScriptedGraphQl, data, operation},
};

const RESET_AT: &str = "2026-11-19T13:00:00Z";

type DetailsAnswer =
    dyn Fn(&GraphQlRequest) -> Result<GraphQlResponse, TransportError> + Send + Sync;

fn page_info(next: bool) -> Value {
    json!({ "hasNextPage": next, "endCursor": if next { json!("more") } else { Value::Null } })
}

fn empty_connection() -> Value {
    json!({ "pageInfo": page_info(false), "nodes": [] })
}

/// The PR as the list query returns it.
fn pr_node(number: u64, mergeable: &str, decision: Value, queued: bool) -> Value {
    json!({
        "id": format!("PR_node_{number}"),
        "number": number,
        "title": format!("PR {number}"),
        "url": format!("https://github.com/octo/app/pull/{number}"),
        "isDraft": false,
        "createdAt": "2026-11-01T00:00:00Z",
        "updatedAt": "2026-11-02T00:00:00Z",
        "author": { "login": "me" },
        "baseRefName": "main",
        "headRefName": format!("feature-{number}"),
        "headRefOid": "abc123",
        "headRepository": { "name": "app", "owner": { "login": "octo" } },
        "mergeable": mergeable,
        "mergeStateStatus": "CLEAN",
        "reviewDecision": decision,
        "isInMergeQueue": queued
    })
}

/// Details with nothing in them.
fn no_details() -> Value {
    json!({
        "headRefOid": "abc123",
        "reviewThreads": empty_connection(),
        "reviewRequests": empty_connection(),
        "latestOpinionatedReviews": empty_connection(),
        "latestReviews": empty_connection(),
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] }
    })
}

/// Details with one of everything.
fn rich_details() -> Value {
    json!({
        "headRefOid": "head1",
        "reviewThreads": { "pageInfo": page_info(false), "nodes": [
            { "isResolved": false }, { "isResolved": true }, { "isResolved": false }
        ]},
        "reviewRequests": { "pageInfo": page_info(false), "nodes": [
            { "requestedReviewer": { "__typename": "User", "login": "alice" } },
            { "requestedReviewer": { "__typename": "Team", "slug": "core", "organization": { "login": "octo" } } }
        ]},
        "latestOpinionatedReviews": { "pageInfo": page_info(false), "nodes": [
            { "state": "CHANGES_REQUESTED", "author": { "__typename": "User", "login": "bob" },
              "commit": { "oid": "c0ffee" } },
            { "state": "APPROVED", "author": null, "commit": null },
            { "state": "APPROVED", "author": { "__typename": "Bot", "login": "ci-bot" },
              "commit": { "oid": "head1" } }
        ]},
        "latestReviews": { "pageInfo": page_info(false), "nodes": [
            { "state": "COMMENTED", "author": { "__typename": "User", "login": "bob" },
              "commit": { "oid": "head1" } },
            { "state": "DISMISSED", "author": { "__typename": "Mannequin", "login": "old" },
              "commit": { "oid": "c0ffee" } }
        ]},
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "contexts": {
            "pageInfo": page_info(false),
            "nodes": [
                { "__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "FAILURE",
                  "isRequired": true, "checkSuite": { "workflowRun": { "workflow": { "name": "CI" } } } },
                { "__typename": "CheckRun", "name": "CodeRabbit", "status": "COMPLETED", "conclusion": "SKIPPED",
                  "isRequired": false, "checkSuite": { "workflowRun": null } },
                { "__typename": "CheckRun", "name": "test", "status": "IN_PROGRESS", "conclusion": null,
                  "isRequired": true, "checkSuite": null },
                { "__typename": "StatusContext", "context": "ci/preview", "state": "ERROR", "isRequired": false },
                { "__typename": "StatusContext", "context": "ci/docs", "state": "EXPECTED", "isRequired": false },
                { "__typename": "CheckRun", "name": "odd", "status": "COMPLETED", "conclusion": "SOMETHING_NEW",
                  "isRequired": false, "checkSuite": null }
            ]
        }}}}]}
    })
}

/// Owner "octo" with one Repo "app" holding `prs`; the details query is
/// answered by `details`.
fn octo(prs: Vec<Value>, details: Box<DetailsAnswer>) -> ScriptedGraphQl {
    octo_with_list_errors(prs, Vec::new(), details)
}

/// As [`octo`], with `list_errors` alongside the PR list's data.
fn octo_with_list_errors(
    prs: Vec<Value>,
    list_errors: Vec<GraphQlError>,
    details: Box<DetailsAnswer>,
) -> ScriptedGraphQl {
    let prs = Arc::new(prs);
    ScriptedGraphQl::new(move |request| match operation(request).as_str() {
        "Viewer" => Ok(data(json!({
            "rateLimit": { "remaining": 4999, "resetAt": RESET_AT },
            "viewer": { "login": "me" }
        }))),
        "ViewerTeams" => Ok(data(json!({ "viewer": { "organizations": {
            "pageInfo": { "hasNextPage": false }, "nodes": []
        }}}))),
        "OwnerRepos" => Ok(data(json!({ "repositoryOwner": { "repositories": {
            "pageInfo": page_info(false),
            "nodes": [{ "name": "app", "url": "https://github.com/octo/app", "isFork": false,
                        "isArchived": false, "defaultBranchRef": { "name": "main" }, "parent": null }]
        }}}))),
        "OwnerPullRequests" => Ok(GraphQlResponse {
            data: Some(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": page_info(false),
                "nodes": [{ "name": "app", "pullRequests": { "pageInfo": page_info(false), "nodes": *prs } }]
            }}})),
            errors: list_errors.clone(),
        }),
        "PullRequestDetails" => details(request),
        "BranchRefs" => only_default_branches(request).expect("a BranchRefs answer"),
        "OwnerIssueCounts" => Ok(data(json!({ "repositoryOwner": { "repositories": {
            "pageInfo": page_info(false), "nodes": []
        }}}))),
        other => panic!("unexpected {other}"),
    })
}

/// Answers every PR in the batch with `node`.
fn each(node: Value) -> Box<DetailsAnswer> {
    Box::new(move |request| {
        let count = request
            .variables
            .as_object()
            .map_or(0, serde_json::Map::len);
        let answer: serde_json::Map<String, Value> = (0..count)
            .map(|i| (format!("pr{i}"), node.clone()))
            .collect();
        Ok(data(Value::Object(answer)))
    })
}

fn the_pr(observation: &Observation, number: u64) -> &PullRequest {
    observation
        .pull_requests
        .iter()
        .find(|p| p.number == number)
        .unwrap_or_else(|| panic!("no PR #{number}"))
}

fn app_prs_state(observation: &Observation) -> Option<SourceState> {
    observation
        .coverage
        .get(&Source::Repo {
            repo: repo_id("octo/app"),
            kind: SourceKind::PullRequests,
        })
        .cloned()
}

fn app_details_state(observation: &Observation) -> Option<SourceState> {
    observation
        .coverage
        .get(&Source::Repo {
            repo: repo_id("octo/app"),
            kind: SourceKind::PullRequestDetails,
        })
        .cloned()
}

fn owner_prs_state(observation: &Observation) -> Option<SourceState> {
    observation
        .coverage
        .get(&Source::Owner {
            owner: "octo".to_owned(),
            kind: SourceKind::PullRequests,
        })
        .cloned()
}

#[test]
fn list_facts_map_mergeability_review_decision_and_merge_queue() {
    let gql = octo(
        vec![
            pr_node(1, "MERGEABLE", json!("APPROVED"), false),
            pr_node(2, "CONFLICTING", json!("CHANGES_REQUESTED"), false),
            pr_node(3, "UNKNOWN", json!("REVIEW_REQUIRED"), true),
            pr_node(4, "SOMETHING_NEW", Value::Null, false),
            pr_node(5, "MERGEABLE", json!("SOMETHING_NEW"), false),
        ],
        each(no_details()),
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let facts: Vec<(Mergeability, ReviewDecision, bool)> = (1..=5)
        .map(|n| {
            let pr = the_pr(&observation, n);
            (pr.mergeability, pr.review_decision, pr.in_merge_queue)
        })
        .collect();
    assert_eq!(
        facts,
        [
            (Mergeability::Mergeable, ReviewDecision::Approved, false),
            (
                Mergeability::Conflicting,
                ReviewDecision::ChangesRequested,
                false
            ),
            (Mergeability::Unknown, ReviewDecision::ReviewRequired, true),
            // A null decision on a page without errors: no review required.
            (Mergeability::Unknown, ReviewDecision::NotRequired, false),
            // A decision den doesn't know is unknown, never "not required".
            (Mergeability::Mergeable, ReviewDecision::Unknown, false),
        ]
    );
    for n in 1..=5 {
        let details = the_pr(&observation, n).details.as_ref().expect("details");
        assert!(details.checks.items.is_empty() && details.checks.complete);
    }
    assert_eq!(owner_prs_state(&observation), Some(SourceState::Complete));
    // A complete first page now records Repo coverage so later Owner failures
    // do not prevent the cache from dropping this Repo's closed PRs.
    assert_eq!(app_prs_state(&observation), Some(SourceState::Complete));
}

#[test]
fn details_map_checks_threads_requests_and_reviews() {
    let gql = octo(
        vec![pr_node(1, "MERGEABLE", Value::Null, false)],
        each(rich_details()),
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let details = the_pr(&observation, 1).details.clone().expect("details");

    let check = |name: &str, required: bool, outcome: CheckOutcome| Check {
        name: name.to_owned(),
        required,
        outcome,
    };
    assert_eq!(
        details.checks.items,
        [
            check("CI / build", true, CheckOutcome::Failing),
            check("CodeRabbit", false, CheckOutcome::Passing),
            check("test", true, CheckOutcome::Pending),
            check("ci/preview", false, CheckOutcome::Failing),
            check("ci/docs", false, CheckOutcome::Pending),
            check(
                "odd",
                false,
                CheckOutcome::Unrecognised("SOMETHING_NEW".to_owned())
            ),
        ]
    );
    assert!(details.checks.complete);
    assert_eq!(details.unresolved_threads, 2);
    assert!(details.threads_complete);
    assert_eq!(
        details.review_requests.items,
        [
            Reviewer::User("alice".to_owned()),
            Reviewer::Team(TeamRef {
                org: "octo".to_owned(),
                slug: "core".to_owned()
            }),
        ]
    );
    assert_eq!(
        details.latest_reviews.items,
        [
            Review {
                author: Some("bob".to_owned()),
                author_kind: ActorKind::User,
                verdict: ReviewVerdict::ChangesRequested,
                commit: Some("c0ffee".to_owned()),
            },
            Review {
                author: None,
                author_kind: ActorKind::Other,
                verdict: ReviewVerdict::Approved,
                commit: None,
            },
            Review {
                author: Some("ci-bot".to_owned()),
                author_kind: ActorKind::Bot,
                verdict: ReviewVerdict::Approved,
                commit: Some("head1".to_owned()),
            },
        ]
    );
    // GitHub's `latestReviews` (#36): the latest review of any kind per
    // reviewer, comments included, with its commit and the author's kind.
    assert_eq!(
        details.latest_reviews_with_comments.items,
        [
            Review {
                author: Some("bob".to_owned()),
                author_kind: ActorKind::User,
                verdict: ReviewVerdict::Commented,
                commit: Some("head1".to_owned()),
            },
            Review {
                author: Some("old".to_owned()),
                author_kind: ActorKind::Other,
                verdict: ReviewVerdict::Other,
                commit: Some("c0ffee".to_owned()),
            },
        ]
    );
    assert!(details.latest_reviews_with_comments.complete);
    // The head as the details query read it, which the reviews' commits are
    // compared against (#36): here a push landed after the PR list (abc123).
    assert_eq!(details.head_commit.as_deref(), Some("head1"));
}

#[test]
fn more_latest_reviews_than_read_stay_a_per_pr_fact_and_leave_coverage_alone() {
    let survey_with = |latest_reviews: Value| {
        let mut node = no_details();
        node["latestReviews"] = latest_reviews;
        let gql = octo(
            vec![pr_node(1, "MERGEABLE", Value::Null, false)],
            each(node),
        );
        survey(&gql, &["octo".to_owned()], now()).expect("survey")
    };
    let carol = json!([
        { "state": "COMMENTED", "author": { "__typename": "User", "login": "carol" },
          "commit": { "oid": "abc" } }
    ]);
    let truncated = survey_with(json!({ "pageInfo": page_info(true), "nodes": carol }));
    let complete = survey_with(json!({ "pageInfo": page_info(false), "nodes": carol }));

    let details = the_pr(&truncated, 1).details.clone().expect("details");
    assert!(!details.latest_reviews_with_comments.complete);
    assert_eq!(details.latest_reviews_with_comments.items.len(), 1);
    // Was `Partial(FollowUpTruncated { "#1 (latest reviews)" })`. Changed on
    // purpose (#51 review): only the Linear Workflow's rules read
    // `latestReviews`, and this source doesn't know which Owners have one, so
    // its truncation is a per-PR fact (above) the workflow rules report, never
    // Repo coverage that would make an Owner without a workflow partial.
    assert_eq!(app_details_state(&truncated), Some(SourceState::Complete));
    assert_eq!(truncated.coverage, complete.coverage);
}

#[test]
fn more_pages_or_unreadable_entries_mark_a_list_incomplete() {
    let mut node = no_details();
    node["reviewThreads"] =
        json!({ "pageInfo": page_info(true), "nodes": [{ "isResolved": false }] });
    node["reviewRequests"] =
        json!({ "pageInfo": page_info(false), "nodes": [{ "requestedReviewer": null }] });
    node["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = json!({ "contexts": {
        "pageInfo": page_info(true),
        "nodes": [{ "__typename": "StatusContext", "context": "a", "state": "SUCCESS", "isRequired": true }]
    }});
    let gql = octo(
        vec![pr_node(1, "MERGEABLE", Value::Null, false)],
        each(node),
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let details = the_pr(&observation, 1).details.clone().expect("details");
    assert!(!details.checks.complete);
    assert_eq!(details.checks.items.len(), 1);
    assert!(!details.threads_complete);
    assert_eq!(details.unresolved_threads, 1);
    assert!(!details.review_requests.complete);
    assert!(details.review_requests.items.is_empty());
    assert!(details.latest_reviews.complete);
    // The response itself was clean, but the header and sidebar read coverage,
    // not these per-PR flags: detail coverage is partial; list coverage stays complete.
    assert_eq!(
        app_details_state(&observation),
        Some(SourceState::Partial(Incompleteness::FollowUpTruncated {
            what: "#1 (checks, review threads, review requests)".to_owned()
        }))
    );
}

#[test]
fn list_facts_map_github_merge_state() {
    let states = [
        ("CLEAN", MergeState::Clean),
        ("HAS_HOOKS", MergeState::HasHooks),
        ("UNSTABLE", MergeState::Unstable),
        ("BLOCKED", MergeState::Blocked),
        ("BEHIND", MergeState::Behind),
        ("DIRTY", MergeState::Dirty),
        ("DRAFT", MergeState::Draft),
        ("UNKNOWN", MergeState::Unknown),
        ("SOMETHING_NEW", MergeState::Unknown),
    ];
    let prs = states
        .iter()
        .zip(1..)
        .map(|((status, _), number)| {
            let mut node = pr_node(number, "MERGEABLE", json!("APPROVED"), false);
            node["mergeStateStatus"] = json!(status);
            node
        })
        .collect();
    let gql = octo(prs, each(no_details()));
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    for ((status, expected), number) in states.iter().zip(1..) {
        assert_eq!(
            the_pr(&observation, number).merge_state,
            *expected,
            "{status}"
        );
    }
}

#[test]
fn a_null_review_decision_on_a_page_with_errors_is_unknown() {
    let gql = octo_with_list_errors(
        vec![
            pr_node(1, "MERGEABLE", Value::Null, false),
            pr_node(2, "MERGEABLE", json!("APPROVED"), false),
        ],
        vec![GraphQlError {
            message: "Something went wrong while executing your query".to_owned(),
            kind: None,
        }],
        each(no_details()),
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    // The error may have nulled PR #1's decision, so it is not "not required".
    assert_eq!(
        the_pr(&observation, 1).review_decision,
        ReviewDecision::Unknown
    );
    assert_eq!(
        the_pr(&observation, 2).review_decision,
        ReviewDecision::Approved
    );
}

#[test]
fn a_null_check_rollup_alongside_errors_is_checks_not_read() {
    let gql = octo(
        vec![pr_node(1, "MERGEABLE", json!("APPROVED"), false)],
        Box::new(|_| {
            Ok(GraphQlResponse {
                data: Some(json!({ "pr0": no_details() })),
                errors: vec![GraphQlError {
                    message: "Something went wrong while executing your query".to_owned(),
                    kind: None,
                }],
            })
        }),
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let checks = &the_pr(&observation, 1)
        .details
        .as_ref()
        .expect("details")
        .checks;
    // Without the error, a null rollup means no checks (see the list test);
    // with it, the rollup may have been nulled, so the checks are unread.
    assert!(checks.items.is_empty());
    assert!(!checks.complete);
}

#[test]
fn details_are_read_in_batches_with_ids_as_variables() {
    let prs = (1..=30)
        .map(|n| pr_node(n, "MERGEABLE", Value::Null, false))
        .collect();
    let gql = octo(prs, each(no_details()));
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert!(
        observation
            .pull_requests
            .iter()
            .all(|p| p.details.is_some())
    );

    let batches: Vec<GraphQlRequest> = gql
        .requests()
        .into_iter()
        .filter(|r| operation(r) == "PullRequestDetails")
        .collect();
    let sizes: Vec<usize> = batches
        .iter()
        .map(|r| r.variables.as_object().map_or(0, serde_json::Map::len))
        .collect();
    assert_eq!(sizes, [25, 5]);
    let first = &batches[0];
    assert_eq!(first.variables["id0"], json!("PR_node_1"));
    assert!(
        !first.query.contains("PR_node_"),
        "IDs never spliced into query text"
    );
    assert!(first.query.contains("isRequired(pullRequestId: $id24)"));
    assert!(!first.query.contains("$PR_ID"));
}

#[test]
fn a_lost_details_query_keeps_the_prs_and_marks_their_repo_partial() {
    let gql = octo(
        vec![
            pr_node(1, "MERGEABLE", json!("APPROVED"), false),
            pr_node(2, "MERGEABLE", Value::Null, false),
        ],
        Box::new(|_| {
            Err(TransportError::TimedOut {
                after: Duration::from_secs(30),
            })
        }),
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(observation.pull_requests.len(), 2);
    assert!(
        observation
            .pull_requests
            .iter()
            .all(|p| p.details.is_none())
    );
    assert_eq!(
        the_pr(&observation, 1).review_decision,
        ReviewDecision::Approved
    );
    assert_eq!(owner_prs_state(&observation), Some(SourceState::Complete));
    // A lost follow-up must not undo the completely read open-PR list.
    assert_eq!(app_prs_state(&observation), Some(SourceState::Complete));
    match app_details_state(&observation) {
        Some(SourceState::Partial(Incompleteness::FollowUpLost { what, reason })) => {
            assert_eq!(what, "checks and reviews of #1, #2");
            assert!(reason.contains("30s"), "{reason}");
        }
        other => panic!("expected a lost follow-up, got {other:?}"),
    }
}

#[test]
fn a_rate_limited_details_query_is_partial_with_the_reset_time() {
    let gql = octo(
        vec![pr_node(1, "MERGEABLE", Value::Null, false)],
        Box::new(|_| {
            Ok(GraphQlResponse {
                data: None,
                errors: vec![GraphQlError {
                    message: "API rate limit exceeded".to_owned(),
                    kind: Some("RATE_LIMITED".to_owned()),
                }],
            })
        }),
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert!(the_pr(&observation, 1).details.is_none());
    assert_eq!(
        app_details_state(&observation),
        Some(SourceState::Partial(Incompleteness::RateLimited {
            resets_at: Some(RESET_AT.parse().expect("timestamp"))
        }))
    );
}

#[test]
fn errors_alongside_details_keep_the_data_and_mark_the_repo_partial() {
    let gql = octo(
        vec![
            pr_node(1, "MERGEABLE", Value::Null, false),
            pr_node(2, "MERGEABLE", Value::Null, false),
        ],
        Box::new(|_| {
            Ok(GraphQlResponse {
                data: Some(json!({ "pr0": no_details(), "pr1": null })),
                errors: vec![GraphQlError {
                    message: "Could not resolve to a node".to_owned(),
                    kind: Some("NOT_FOUND".to_owned()),
                }],
            })
        }),
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert!(the_pr(&observation, 1).details.is_some());
    assert!(the_pr(&observation, 2).details.is_none());
    assert_eq!(
        app_details_state(&observation),
        Some(SourceState::Partial(Incompleteness::ErrorsAlongsideData {
            errors: vec!["Could not resolve to a node".to_owned()]
        }))
    );
}

#[test]
fn unread_details_do_not_keep_closed_prs_but_an_incomplete_list_does() {
    use den::cache::{self, CachedObservation};
    let mut truncated = no_details();
    truncated["reviewThreads"]["pageInfo"] = page_info(true);
    let cases: Vec<Box<DetailsAnswer>> = vec![
        each(truncated),
        Box::new(|_| {
            Err(TransportError::TimedOut {
                after: Duration::from_secs(30),
            })
        }),
        Box::new(|_| {
            Ok(GraphQlResponse {
                data: Some(json!({ "pr0": no_details() })),
                errors: vec![GraphQlError {
                    message: "unread check".to_owned(),
                    kind: None,
                }],
            })
        }),
        Box::new(|_| {
            Ok(GraphQlResponse {
                data: None,
                errors: vec![GraphQlError {
                    message: "API rate limit exceeded".to_owned(),
                    kind: Some("RATE_LIMITED".to_owned()),
                }],
            })
        }),
    ];
    for details in cases {
        let fresh = survey(
            &octo(vec![pr_node(1, "MERGEABLE", Value::Null, false)], details),
            &["octo".to_owned()],
            now(),
        )
        .unwrap();
        assert_eq!(app_prs_state(&fresh), Some(SourceState::Complete));
        assert!(!app_details_state(&fresh).unwrap().is_complete());
        assert!(
            fresh
                .coverage
                .applicable_to(&the_pr(&fresh, 1).id())
                .iter()
                .any(|entry| !entry.state.is_complete())
        );
        let mut earlier = fresh.clone();
        let mut closed = earlier.pull_requests[0].clone();
        closed.number = 999;
        earlier.pull_requests.push(closed);
        let earlier = CachedObservation::fresh(earlier);
        let merged = cache::merge(Some(earlier.clone()), fresh.clone()).cached;
        assert_eq!(merged.observation.pull_requests.len(), 1);
        assert!(!merged.observation.coverage.is_complete());

        let mut partial = fresh;
        partial.coverage.record(
            Source::Repo {
                repo: repo_id("octo/app"),
                kind: SourceKind::PullRequests,
            },
            SourceState::Partial(Incompleteness::PaginationStopped {
                pages_read: 1,
                reason: "lost page".to_owned(),
            }),
        );
        let merged = cache::merge(Some(earlier), partial).cached;
        assert_eq!(
            merged.observation.pull_requests.len(),
            2,
            "unread list pages still retain cached PRs"
        );
    }
}
