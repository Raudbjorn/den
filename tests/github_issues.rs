//! The real GitHub source reading Issues and their linked PRs, over a scripted
//! GraphQL transport. Thin adapter checks (spec, Testing Decisions); the rules
//! are tested at seam 1 in `decide_issues.rs`.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{
        Incompleteness, Observation, Source, SourceKind, SourceState,
        facts::{Issue, IssueLink, LinkKind, LinkedPrState},
    },
    github::{
        graphql::{GraphQlRequest, GraphQlResponse, TransportError},
        survey,
    },
};
use serde_json::{Value, json};
use std::time::Duration;
use support::{
    build::{now, repo_id},
    fakes::{ScriptedGraphQl, data, operation},
};

fn page_info(next: Option<&str>) -> Value {
    json!({ "hasNextPage": next.is_some(), "endCursor": next })
}

fn after(request: &GraphQlRequest) -> Option<String> {
    request.variables["after"].as_str().map(str::to_owned)
}

fn repo_name(request: &GraphQlRequest) -> String {
    request.variables["name"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn empty_owner() -> Result<GraphQlResponse, TransportError> {
    Ok(data(json!({ "repositoryOwner": { "repositories": {
        "pageInfo": page_info(None), "nodes": []
    }}})))
}

/// Answers the viewer and every non-Issue query with "nothing".
fn non_issue_answers(request: &GraphQlRequest) -> Option<Result<GraphQlResponse, TransportError>> {
    match operation(request).as_str() {
        "Viewer" => Some(Ok(data(json!({ "viewer": { "login": "me" } })))),
        "ViewerTeams" => Some(Ok(data(json!({ "viewer": { "organizations": {
            "pageInfo": { "hasNextPage": false }, "nodes": []
        }}})))),
        "OwnerRepos" | "OwnerPullRequests" => Some(empty_owner()),
        _ => None,
    }
}

fn linked(number: u64, state: &str, owner: &str, name: &str) -> Value {
    json!({ "number": number, "state": state, "repository": { "name": name, "owner": { "login": owner } } })
}

fn links(nodes: Vec<Value>, more: bool) -> Value {
    json!({ "pageInfo": { "hasNextPage": more, "endCursor": if more { json!("c") } else { Value::Null } }, "nodes": nodes })
}

fn issue_node(number: u64, closing: Value, manual: Value) -> Value {
    json!({
        "number": number,
        "title": format!("Issue {number}"),
        "url": format!("https://github.com/octo/x/issues/{number}"),
        "updatedAt": "2026-11-02T00:00:00Z",
        "labels": { "nodes": [{ "name": "bug" }] },
        "assignees": { "nodes": [{ "login": "me" }, { "login": "alice" }] },
        "closingReferences": closing,
        "manualLinks": manual
    })
}

fn plain_issue(number: u64) -> Value {
    issue_node(number, links(vec![], false), links(vec![], false))
}

fn counts(nodes: Vec<(&str, u64)>, next: Option<&str>) -> Result<GraphQlResponse, TransportError> {
    let nodes: Vec<Value> = nodes
        .into_iter()
        .map(|(name, open)| json!({ "name": name, "issues": { "totalCount": open } }))
        .collect();
    Ok(data(json!({ "repositoryOwner": { "repositories": {
        "pageInfo": page_info(next), "nodes": nodes
    }}})))
}

fn issues_page(nodes: Vec<Value>, next: Option<&str>) -> Result<GraphQlResponse, TransportError> {
    Ok(data(json!({ "repository": { "issues": {
        "pageInfo": page_info(next), "nodes": nodes
    }}})))
}

/// Owner "octo": Repo counts over two pages; "busy" has two pages of Issues,
/// "linked" has one Issue with links of both kinds in several Repos, "quiet"
/// and "empty" have none.
fn octo(request: &GraphQlRequest) -> Result<GraphQlResponse, TransportError> {
    if let Some(answer) = non_issue_answers(request) {
        return answer;
    }
    match (
        operation(request).as_str(),
        repo_name(request).as_str(),
        after(request).as_deref(),
    ) {
        ("OwnerIssueCounts", _, None) => counts(vec![("busy", 3), ("empty", 0)], Some("c2")),
        ("OwnerIssueCounts", _, Some("c2")) => counts(vec![("linked", 1), ("quiet", 0)], None),
        ("RepoIssues", "busy", None) => {
            issues_page(vec![plain_issue(1), plain_issue(2)], Some("b2"))
        }
        ("RepoIssues", "busy", Some("b2")) => issues_page(vec![plain_issue(3)], None),
        ("RepoIssues", "linked", None) => issues_page(
            vec![issue_node(
                7,
                links(
                    vec![
                        linked(10, "OPEN", "octo", "linked"),
                        linked(11, "CLOSED", "octo", "linked"),
                    ],
                    false,
                ),
                links(
                    vec![
                        linked(77, "OPEN", "elsewhere", "lib"),
                        linked(5, "MERGED", "octo", "busy"),
                    ],
                    false,
                ),
            )],
            None,
        ),
        other => panic!("unexpected request {other:?}"),
    }
}

fn state(observation: &Observation, source: Source) -> SourceState {
    observation
        .coverage
        .get(&source)
        .cloned()
        .unwrap_or_else(|| panic!("no coverage for {source:?}: {:?}", observation.coverage))
}

fn issues_of(repo: &str) -> Source {
    Source::Repo {
        repo: repo_id(repo),
        kind: SourceKind::Issues,
    }
}

fn find<'a>(observation: &'a Observation, repo: &str, number: u64) -> &'a Issue {
    observation
        .issues
        .iter()
        .find(|i| i.repo == repo_id(repo) && i.number == number)
        .unwrap_or_else(|| panic!("no Issue {repo}#{number}"))
}

#[test]
fn open_issues_are_read_only_for_repos_that_have_them_and_paginated_to_completion() {
    let gql = ScriptedGraphQl::new(octo);
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");

    let mut issues: Vec<(String, u64)> = observation
        .issues
        .iter()
        .map(|i| (i.repo.to_string(), i.number))
        .collect();
    issues.sort();
    assert_eq!(
        issues,
        [
            ("octo/busy".to_owned(), 1),
            ("octo/busy".to_owned(), 2),
            ("octo/busy".to_owned(), 3),
            ("octo/linked".to_owned(), 7),
        ]
    );
    let repos_read: Vec<String> = gql
        .requests()
        .iter()
        .filter(|r| operation(r) == "RepoIssues")
        .map(repo_name)
        .collect();
    assert_eq!(repos_read, ["busy", "busy", "linked"]);

    let busy = find(&observation, "octo/busy", 1);
    assert_eq!(busy.labels, ["bug"]);
    assert_eq!(busy.assignees, ["me", "alice"]);
    assert!(busy.linked_prs.is_empty());
    assert!(busy.linked_prs_complete);

    for source in [
        Source::Owner {
            owner: "octo".to_owned(),
            kind: SourceKind::Issues,
        },
        issues_of("octo/busy"),
        issues_of("octo/linked"),
    ] {
        assert_eq!(
            state(&observation, source.clone()),
            SourceState::Complete,
            "{source:?}"
        );
    }
}

#[test]
fn linked_prs_carry_their_relationship_repo_and_state() {
    let gql = ScriptedGraphQl::new(octo);
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let issue = find(&observation, "octo/linked", 7);
    let link = |kind, repo: &str, number, state| IssueLink {
        kind,
        repo: repo_id(repo),
        number,
        state,
    };
    assert_eq!(
        issue.linked_prs,
        [
            link(
                LinkKind::ClosingReference,
                "octo/linked",
                10,
                LinkedPrState::Open
            ),
            link(
                LinkKind::ClosingReference,
                "octo/linked",
                11,
                LinkedPrState::Closed
            ),
            link(
                LinkKind::ManualLink,
                "elsewhere/lib",
                77,
                LinkedPrState::Open
            ),
            link(LinkKind::ManualLink, "octo/busy", 5, LinkedPrState::Merged),
        ]
    );
    assert!(issue.linked_prs_complete);
}

#[test]
fn the_query_reads_both_link_kinds_in_every_state_and_no_mentions() {
    let gql = ScriptedGraphQl::new(octo);
    survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let requests = gql.requests();
    let query = &requests
        .iter()
        .find(|r| operation(r) == "RepoIssues")
        .expect("a RepoIssues request")
        .query;
    // Closing references and manual links partition closedByPullRequestsReferences;
    // without includeClosedPrs GitHub drops closed PRs, so their state couldn't be shown.
    for needle in [
        "closingReferences: closedByPullRequestsReferences(first: 10, includeClosedPrs: true, excludeUserLinked: true)",
        "manualLinks: closedByPullRequestsReferences(first: 10, includeClosedPrs: true, userLinkedOnly: true)",
    ] {
        assert!(query.contains(needle), "{needle} missing from {query}");
    }
    // Mentions arrive as cross-reference timeline events; den never reads them.
    for mention_source in ["timelineItems", "CrossReferencedEvent"] {
        assert!(!query.contains(mention_source), "{mention_source}");
    }
}

#[test]
fn issue_pages_are_ordered_by_a_key_that_cannot_change_mid_read() {
    let gql = ScriptedGraphQl::new(octo);
    survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let requests = gql.requests();
    let query = &requests
        .iter()
        .find(|r| operation(r) == "RepoIssues")
        .expect("a RepoIssues request")
        .query;
    // Cursors are positions in the sort order. Creation time never changes, so an
    // Issue can't move ahead of the cursor mid-read and be skipped, as it could
    // under UPDATED_AT.
    assert!(
        query.contains("orderBy: {field: CREATED_AT, direction: DESC}"),
        "{query}"
    );
    assert!(!query.contains("UPDATED_AT"), "{query}");
}

#[test]
fn more_links_than_read_or_unreadable_links_mark_the_links_incomplete() {
    let gql = ScriptedGraphQl::new(|request| {
        if let Some(answer) = non_issue_answers(request) {
            return answer;
        }
        match operation(request).as_str() {
            "OwnerIssueCounts" => counts(vec![("app", 3)], None),
            "RepoIssues" => Ok(GraphQlResponse {
                data: Some(json!({ "repository": { "issues": {
                    "pageInfo": page_info(None),
                    "nodes": [
                        issue_node(1, links(vec![linked(2, "CLOSED", "octo", "app")], true), links(vec![], false)),
                        issue_node(3, links(vec![], false), links(vec![Value::Null], false)),
                        issue_node(4, Value::Null, links(vec![], false)),
                    ]
                }}})),
                errors: vec![den::github::graphql::GraphQlError {
                    message: "Resource not accessible by integration".to_owned(),
                    kind: Some("FORBIDDEN".to_owned()),
                }],
            }),
            other => panic!("unexpected {other}"),
        }
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    for number in [1, 3, 4] {
        assert!(
            !find(&observation, "octo/app", number).linked_prs_complete,
            "#{number}"
        );
    }
    assert!(matches!(
        state(&observation, issues_of("octo/app")),
        SourceState::Partial(_)
    ));
}

/// GitHub says there are more linked PRs than den asked for, with no error at
/// all: den stopped reading them on purpose, so the Repo's Issues are partial,
/// naming the Issues whose links were cut short (an unread open PR could take
/// the Issue's Next Step away).
#[test]
fn links_cut_short_without_any_error_make_the_repos_issues_partial() {
    let gql = ScriptedGraphQl::new(|request| {
        if let Some(answer) = non_issue_answers(request) {
            return answer;
        }
        match operation(request).as_str() {
            "OwnerIssueCounts" => counts(vec![("app", 3)], None),
            "RepoIssues" => issues_page(
                vec![
                    issue_node(
                        1,
                        links(vec![linked(2, "CLOSED", "octo", "app")], true),
                        links(vec![], false),
                    ),
                    plain_issue(3),
                    issue_node(
                        5,
                        links(vec![], false),
                        links(vec![linked(6, "OPEN", "octo", "app")], true),
                    ),
                ],
                None,
            ),
            other => panic!("unexpected {other}"),
        }
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert!(!find(&observation, "octo/app", 1).linked_prs_complete);
    assert!(find(&observation, "octo/app", 3).linked_prs_complete);
    assert!(!find(&observation, "octo/app", 5).linked_prs_complete);
    assert_eq!(
        state(&observation, issues_of("octo/app")),
        SourceState::Partial(Incompleteness::PaginationStopped {
            pages_read: 1,
            reason: "more linked PRs than den reads on #1, #5".to_owned(),
        })
    );
}

#[test]
fn unreadable_labels_or_assignees_are_marked_unread_not_read_as_none() {
    let gql = ScriptedGraphQl::new(|request| {
        if let Some(answer) = non_issue_answers(request) {
            return answer;
        }
        let with = |number: u64, field: &str, value: Value| {
            let mut node = plain_issue(number);
            node[field] = value;
            node
        };
        match operation(request).as_str() {
            "OwnerIssueCounts" => counts(vec![("app", 4)], None),
            "RepoIssues" => Ok(GraphQlResponse {
                data: Some(json!({ "repository": { "issues": {
                    "pageInfo": page_info(None),
                    "nodes": [
                        plain_issue(1),
                        with(2, "labels", Value::Null),
                        with(3, "assignees", json!({ "nodes": [{ "login": "me" }, Value::Null] })),
                        with(4, "labels", json!({ "nodes": [Value::Null] })),
                    ]
                }}})),
                errors: vec![den::github::graphql::GraphQlError {
                    message: "Resource not accessible by integration".to_owned(),
                    kind: Some("FORBIDDEN".to_owned()),
                }],
            }),
            other => panic!("unexpected {other}"),
        }
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let read = |number| {
        let issue = find(&observation, "octo/app", number);
        (issue.labels_complete, issue.assignees_complete)
    };
    assert_eq!(read(1), (true, true));
    assert_eq!(read(2), (false, true));
    assert_eq!(read(3), (true, false));
    assert_eq!(read(4), (false, true));
    assert_eq!(find(&observation, "octo/app", 3).assignees, ["me"]);
}

#[test]
fn a_repo_whose_issues_cannot_be_read_fails_alone() {
    let gql = ScriptedGraphQl::new(|request| {
        if let Some(answer) = non_issue_answers(request) {
            return answer;
        }
        match (operation(request).as_str(), repo_name(request).as_str()) {
            ("OwnerIssueCounts", _) => counts(vec![("broken", 2), ("fine", 1)], None),
            ("RepoIssues", "broken") => Err(TransportError::TimedOut {
                after: Duration::from_secs(30),
            }),
            ("RepoIssues", "fine") => issues_page(vec![plain_issue(1)], None),
            other => panic!("unexpected {other:?}"),
        }
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(observation.issues.len(), 1);
    assert!(matches!(
        state(&observation, issues_of("octo/broken")),
        SourceState::Failed { .. }
    ));
    assert_eq!(
        state(&observation, issues_of("octo/fine")),
        SourceState::Complete
    );
    assert_eq!(
        state(
            &observation,
            Source::Owner {
                owner: "octo".to_owned(),
                kind: SourceKind::Issues
            }
        ),
        SourceState::Complete
    );
}

#[test]
fn an_outage_stops_the_issue_reads_instead_of_timing_out_once_per_repo() {
    // Two Repos in a row unreachable (each a whole `gh` timeout in real life):
    // the rest are not asked about, and their coverage says why.
    let gql = ScriptedGraphQl::new(|request| {
        if let Some(answer) = non_issue_answers(request) {
            return answer;
        }
        match (operation(request).as_str(), repo_name(request).as_str()) {
            ("OwnerIssueCounts", _) => {
                counts(vec![("a", 1), ("b", 1), ("c", 1), ("d", 1), ("e", 1)], None)
            }
            ("RepoIssues", "a" | "c" | "d") => Err(TransportError::TimedOut {
                after: Duration::from_secs(30),
            }),
            ("RepoIssues", "b") => issues_page(vec![plain_issue(1)], None),
            other => panic!("unexpected {other:?}"),
        }
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");

    // "a" alone is not an outage: "b" is still read, and resets the count.
    let repos_read: Vec<String> = gql
        .requests()
        .iter()
        .filter(|r| operation(r) == "RepoIssues")
        .map(repo_name)
        .collect();
    assert_eq!(repos_read, ["a", "b", "c", "d"]);
    assert_eq!(
        state(&observation, issues_of("octo/b")),
        SourceState::Complete
    );
    let skipped = state(&observation, issues_of("octo/e"));
    assert!(
        matches!(&skipped, SourceState::Failed { error } if error.contains("not read") && error.contains("did not answer")),
        "{skipped:?}"
    );
}

#[test]
fn once_rate_limited_the_remaining_repos_are_rate_limited_not_asked() {
    let gql = ScriptedGraphQl::new(|request| {
        if let Some(answer) = non_issue_answers(request) {
            return answer;
        }
        match operation(request).as_str() {
            "OwnerIssueCounts" => counts(vec![("a", 1), ("b", 1), ("c", 1)], None),
            "RepoIssues" => Err(TransportError::RateLimited {
                detail: "API rate limit exceeded".to_owned(),
            }),
            other => panic!("unexpected {other}"),
        }
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    let asked = gql
        .requests()
        .iter()
        .filter(|r| operation(r) == "RepoIssues")
        .count();
    assert_eq!(asked, 2);
    assert!(matches!(
        state(&observation, issues_of("octo/c")),
        SourceState::Partial(Incompleteness::RateLimited { .. })
    ));
}

#[test]
fn an_unknown_owner_fails_its_issues_source() {
    let gql = ScriptedGraphQl::new(|request| {
        non_issue_answers(request).unwrap_or_else(|| {
            Ok(GraphQlResponse {
                data: Some(json!({ "repositoryOwner": null })),
                errors: Vec::new(),
            })
        })
    });
    let observation = survey(&gql, &["nobody-here".to_owned()], now()).expect("survey");
    let state = state(
        &observation,
        Source::Owner {
            owner: "nobody-here".to_owned(),
            kind: SourceKind::Issues,
        },
    );
    assert!(
        matches!(&state, SourceState::Failed { error } if error.contains("nobody-here")),
        "{state:?}"
    );
}
