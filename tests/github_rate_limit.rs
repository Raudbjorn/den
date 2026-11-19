//! The GitHub source under a rate limit (thin adapter checks over a scripted
//! transport): once GitHub says the limit was hit, the survey stops fetching,
//! records the rest as partial, and says when the limit resets.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{Incompleteness, Source, SourceKind, SourceState},
    github::{
        graphql::{GraphQl, GraphQlError, GraphQlRequest, GraphQlResponse, TransportError},
        survey,
    },
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde_json::{Value, json};
use std::sync::Mutex;
use support::{
    build::{now, ts},
    fakes::{data, operation},
};

const RESET_AT: &str = "2026-11-19T12:45:00Z";

/// Answers by operation name, records the operations asked, and answers the
/// "when does the limit reset?" question with `reset`.
struct Transport {
    answer: fn(&str) -> Result<GraphQlResponse, TransportError>,
    reset: Option<Timestamp>,
    operations: Mutex<Vec<String>>,
    reset_questions: Mutex<u32>,
}

impl Transport {
    fn new(answer: fn(&str) -> Result<GraphQlResponse, TransportError>) -> Self {
        Self {
            answer,
            reset: Some(ts(RESET_AT)),
            operations: Mutex::default(),
            reset_questions: Mutex::default(),
        }
    }

    fn operations(&self) -> Vec<String> {
        self.operations.lock().unwrap().clone()
    }
}

impl GraphQl for Transport {
    fn execute(&self, request: &GraphQlRequest) -> Result<GraphQlResponse, TransportError> {
        let op = operation(request);
        self.operations.lock().unwrap().push(op.clone());
        (self.answer)(&op)
    }

    fn rate_limit_reset(&self) -> Option<Timestamp> {
        *self.reset_questions.lock().unwrap() += 1;
        self.reset
    }
}

/// HTTP 200 with a RATE_LIMITED error and no data, as GitHub sends it.
fn rate_limited_body() -> Result<GraphQlResponse, TransportError> {
    Ok(GraphQlResponse {
        data: Some(Value::Null),
        errors: vec![GraphQlError {
            message: "API rate limit exceeded for user ID 1.".to_owned(),
            kind: Some("RATE_LIMITED".to_owned()),
        }],
    })
}

fn viewer(op: &str) -> Option<Result<GraphQlResponse, TransportError>> {
    match op {
        "Viewer" => Some(Ok(data(json!({ "viewer": { "login": "me" } })))),
        "ViewerTeams" => Some(Ok(data(json!({ "viewer": { "organizations": {
            "pageInfo": { "hasNextPage": false }, "nodes": []
        }}})))),
        _ => None,
    }
}

#[test]
fn after_a_rate_limit_nothing_more_is_fetched_and_the_rest_is_partial() {
    let gql = Transport::new(|op| match viewer(op) {
        Some(answer) => answer,
        None if op == "OwnerRepos" => rate_limited_body(),
        None => panic!("{op} was sent after the rate limit"),
    });
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("an Observation");

    assert_eq!(gql.operations(), ["Viewer", "ViewerTeams", "OwnerRepos"]);
    let expected = SourceState::Partial(Incompleteness::RateLimited {
        resets_at: Some(ts(RESET_AT)),
    });
    for kind in [SourceKind::Repos, SourceKind::PullRequests] {
        let source = Source::Owner {
            owner: "octo".to_owned(),
            kind,
        };
        assert_eq!(
            observation.coverage.get(&source),
            Some(&expected),
            "{kind:?}"
        );
    }
    assert_eq!(
        *gql.reset_questions.lock().unwrap(),
        1,
        "asked once when it resets"
    );
}

#[test]
fn a_rate_limit_on_the_viewer_is_a_rate_limited_failure_with_the_reset_time() {
    let gql = Transport::new(|_| rate_limited_body());
    assert_eq!(
        survey(&gql, &["octo".to_owned()], now()),
        Err(SurveyFailure::RateLimited {
            resets_at: Some(ts(RESET_AT))
        })
    );

    let mut unknown = Transport::new(|_| {
        Err(TransportError::RateLimited {
            detail: "gh: API rate limit exceeded".to_owned(),
        })
    });
    unknown.reset = None;
    assert_eq!(
        survey(&unknown, &["octo".to_owned()], now()),
        Err(SurveyFailure::RateLimited { resets_at: None })
    );
}
