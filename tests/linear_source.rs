//! Adapter checks for the Linear source (#32), through a scripted transport:
//! no network. Paging across pages, the page limit (partial), errors next to
//! data (partial), a rate limit on HTTP 200 or 429 (partial with its reset),
//! a refused key or no network (failed), and the key read only from its
//! variable. (That the key never reaches the log is `linear_key_logging.rs`.)

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    config::LinearWorkflow,
    domain::{
        Incompleteness, LinearIssueId, LinearSourceKind, Source, SourceKind, SourceState,
        coverage::MAX_KEPT_LINEAR_ERRORS,
        facts::{LinearPriority, LinearStateKind},
    },
    linear::{ApiKey, HttpReply, LinearApi, LinearRequest, MAX_LINEAR_PAGES, TransportError},
    ports::{LinearSource, LinearSurvey, LinearSurveyRequest},
};
use jiff::SignedDuration;
use serde_json::json;
use std::{
    ffi::OsString,
    sync::{Arc, Mutex},
    time::Duration,
};
use support::{
    build::now,
    fakes::{FixedClock, ScriptedLinear, SharedLinear},
    harness::linear_workflow,
    linear_replies::{
        KEY, KEY_ENV, is_viewer, issue_node, issues_page, ok, reply, source, viewer_body,
    },
};

fn survey_with(
    script: impl Fn(&LinearRequest) -> Result<HttpReply, TransportError> + Send + Sync + 'static,
) -> (LinearSurvey, Arc<ScriptedLinear>) {
    let transport = Arc::new(ScriptedLinear::new(script));
    let survey = source(transport.clone(), Arc::default()).survey(&request());
    (survey, transport)
}

fn request() -> LinearSurveyRequest {
    LinearSurveyRequest {
        workflows: vec![workflow()],
    }
}

fn workflow() -> LinearWorkflow {
    linear_workflow("acme", "acme")
}

fn state(survey: &LinearSurvey, kind: LinearSourceKind) -> SourceState {
    survey
        .coverage
        .get(&Source::Owner {
            owner: "acme".to_owned(),
            kind: SourceKind::Linear(kind),
        })
        .cloned()
        .unwrap_or_else(|| panic!("{kind:?} recorded: {:?}", survey.coverage))
}

#[test]
fn reads_the_viewer_and_every_page_of_assigned_issues() {
    let (survey, transport) = survey_with(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        Ok(match request.variables["after"].as_str() {
            None => ok(issues_page(
                vec![issue_node("id-1", "TEAM-1"), issue_node("id-2", "TEAM-2")],
                Some("cursor-1"),
            )),
            Some("cursor-1") => ok(issues_page(vec![issue_node("id-3", "TEAM-3")], None)),
            Some(other) => panic!("unexpected cursor {other}"),
        })
    });

    assert_eq!(
        state(&survey, LinearSourceKind::Viewer),
        SourceState::Complete
    );
    assert_eq!(
        state(&survey, LinearSourceKind::AssignedIssues),
        SourceState::Complete
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 3, "viewer + two pages");
    assert!(
        requests
            .iter()
            .all(|r| r.query.trim_start().starts_with("query")),
        "read-only: queries only"
    );

    let viewer = &survey.part.viewers[0];
    assert_eq!(
        (
            viewer.owner.as_str(),
            viewer.workspace.as_str(),
            viewer.id.as_str()
        ),
        ("acme", "acme", "user-me")
    );
    let identifiers: Vec<&str> = survey
        .part
        .issues
        .iter()
        .map(|i| i.identifier.as_str())
        .collect();
    assert_eq!(identifiers, ["TEAM-1", "TEAM-2", "TEAM-3"]);
    let first = &survey.part.issues[0];
    assert_eq!(first.id, LinearIssueId::new("acme", "id-1"));
    assert_eq!(first.owner, "acme");
    assert_eq!(first.team.key, "TEAM");
    assert_eq!(first.state.kind, LinearStateKind::Started);
    assert_eq!(first.priority, LinearPriority::High);
    assert_eq!(first.assignee.as_ref().unwrap().id, "user-me");
    let cycle = first.cycle.as_ref().unwrap();
    assert!(cycle.active);
    assert_eq!(cycle.number, 3);
    assert_eq!(first.branch_name.as_deref(), Some("me/team-1-do"));
}

#[test]
fn stopping_at_the_page_limit_is_partial() {
    let (survey, transport) = survey_with(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        let page = request.variables["after"]
            .as_str()
            .map_or(0, |c| c.parse::<u32>().unwrap());
        let id = format!("id-{page}");
        let next = (page + 1).to_string();
        Ok(ok(issues_page(
            vec![issue_node(&id, &format!("TEAM-{page}"))],
            Some(&next),
        )))
    });
    let pages = transport.requests().len() - 1;
    assert_eq!(pages, usize::try_from(MAX_LINEAR_PAGES).unwrap());
    assert_eq!(
        survey.part.issues.len(),
        usize::try_from(MAX_LINEAR_PAGES).unwrap(),
        "what was read is kept"
    );
    assert!(
        matches!(
            state(&survey, LinearSourceKind::AssignedIssues),
            SourceState::Partial(Incompleteness::PaginationStopped { pages_read, .. })
                if pages_read == MAX_LINEAR_PAGES
        ),
        "{:?}",
        state(&survey, LinearSourceKind::AssignedIssues)
    );
}

#[test]
fn errors_next_to_data_are_partial() {
    let (survey, _) = survey_with(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        let mut body = issues_page(vec![issue_node("id-1", "TEAM-1")], None);
        body["errors"] = json!([{ "message": "cycle could not be resolved", "path": ["viewer"] }]);
        Ok(ok(body))
    });
    assert_eq!(survey.part.issues.len(), 1);
    assert_eq!(
        state(&survey, LinearSourceKind::AssignedIssues),
        SourceState::Partial(Incompleteness::LinearErrorsAlongsideData {
            errors: vec!["cycle could not be resolved".to_owned()],
            // The new count of messages not kept: none, with one error.
            more: 0,
        })
    );
}

#[test]
fn many_errors_next_to_data_keep_a_few_messages_and_count_the_rest() {
    const SENT_PER_PAGE: usize = 2000;
    let (survey, _) = survey_with(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        let next = request.variables["after"].is_null().then_some("cursor-1");
        let mut body = issues_page(vec![issue_node("id-1", "TEAM-1")], next);
        let errors: Vec<_> = (0..SENT_PER_PAGE)
            .map(|i| json!({ "message": format!("error {i}: {}", "x".repeat(250)) }))
            .collect();
        body["errors"] = json!(errors);
        Ok(ok(body))
    });
    let SourceState::Partial(why @ Incompleteness::LinearErrorsAlongsideData { .. }) =
        state(&survey, LinearSourceKind::AssignedIssues)
    else {
        panic!("partial with errors alongside data");
    };
    let Incompleteness::LinearErrorsAlongsideData { errors, more } = &why else {
        unreachable!()
    };
    assert_eq!(errors.len(), MAX_KEPT_LINEAR_ERRORS, "only a few are kept");
    assert_eq!(
        errors.len() + more,
        2 * SENT_PER_PAGE,
        "every error of both pages is counted"
    );
    assert!(errors[0].starts_with("error 0:"), "the first are kept");
    assert!(
        why.describe()
            .starts_with(&format!("Linear returned {} errors", 2 * SENT_PER_PAGE)),
        "{}",
        why.describe()
    );
}

#[test]
fn a_reply_with_only_errors_keeps_a_few_messages_in_its_failure() {
    let (survey, _) = survey_with(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        let errors: Vec<_> = (0..1000)
            .map(|i| json!({ "message": format!("error {i}") }))
            .collect();
        Ok(ok(json!({ "data": null, "errors": errors })))
    });
    match state(&survey, LinearSourceKind::AssignedIssues) {
        SourceState::Failed { error } => {
            assert!(error.len() < 1000, "bounded: {error}");
            assert!(
                error.contains("error 0") && error.ends_with("and 995 more"),
                "{error}"
            );
        }
        other => panic!("failed, not {other:?}"),
    }
}

#[test]
fn a_rate_limited_http_200_is_partial_with_its_reset_and_stops_asking() {
    let reset = now() + SignedDuration::from_mins(20);
    let reset_ms = reset.as_millisecond().to_string();
    let (survey, transport) = survey_with(move |request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        if request.variables["after"].is_null() {
            return Ok(ok(issues_page(
                vec![issue_node("id-1", "TEAM-1")],
                Some("cursor-1"),
            )));
        }
        Ok(reply(
            200,
            json!({ "errors": [{ "message": "Rate limit exceeded",
                                  "extensions": { "code": "RATELIMITED" } }] }),
            &[("x-ratelimit-requests-reset", &reset_ms)],
        ))
    });
    assert_eq!(
        state(&survey, LinearSourceKind::AssignedIssues),
        SourceState::Partial(Incompleteness::LinearRateLimited {
            resets_at: Some(reset),
        })
    );
    assert_eq!(survey.part.issues.len(), 1, "the first page is kept");
    assert_eq!(
        transport.requests().len(),
        3,
        "nothing sent after the limit"
    );
}

#[test]
fn http_429_with_retry_after_is_partial_with_its_reset() {
    let (survey, _) = survey_with(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        Ok(reply(
            429,
            json!({ "errors": [{ "message": "Too many requests",
                                  "extensions": { "code": "RATELIMITED" } }] }),
            &[("retry-after", "90")],
        ))
    });
    assert_eq!(
        state(&survey, LinearSourceKind::AssignedIssues),
        SourceState::Partial(Incompleteness::LinearRateLimited {
            resets_at: Some(now() + SignedDuration::from_secs(90)),
        })
    );
}

#[test]
fn a_refused_key_fails_both_sources_naming_the_variable() {
    for refused in [
        reply(
            401,
            json!({ "errors": [{ "message": "Authentication required" }] }),
            &[],
        ),
        reply(
            400,
            json!({ "errors": [{ "message": "Invalid API key",
                                  "extensions": { "code": "AUTHENTICATION_ERROR" } }] }),
            &[],
        ),
    ] {
        let (survey, transport) = survey_with(move |_| Ok(refused.clone()));
        for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
            match state(&survey, kind) {
                SourceState::Failed { error } => {
                    assert!(error.contains(KEY_ENV), "{error}");
                    assert!(!error.contains(KEY), "{error}");
                }
                other => panic!("{kind:?} failed, not {other:?}"),
            }
        }
        assert_eq!(
            transport.requests().len(),
            1,
            "no issue query after a refusal"
        );
        assert!(survey.part.viewers.is_empty());
    }
}

#[test]
fn no_network_fails_both_sources() {
    let (survey, _) = survey_with(|_| {
        Err(TransportError::TimedOut {
            after: Duration::from_secs(20),
        })
    });
    for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
        assert!(
            matches!(state(&survey, kind), SourceState::Failed { error } if error.contains("did not answer")),
            "{kind:?}"
        );
    }
}

#[test]
fn a_key_for_another_workspace_fails_naming_both() {
    let (survey, transport) = survey_with(|_| Ok(ok(viewer_body("someone-else"))));
    match state(&survey, LinearSourceKind::Viewer) {
        SourceState::Failed { error } => {
            assert!(
                error.contains("someone-else") && error.contains("acme"),
                "{error}"
            );
        }
        other => panic!("failed, not {other:?}"),
    }
    assert!(matches!(
        state(&survey, LinearSourceKind::AssignedIssues),
        SourceState::Failed { .. }
    ));
    assert!(survey.part.viewers.is_empty(), "not this Owner's viewer");
    assert_eq!(transport.requests().len(), 1);
}

#[test]
fn a_missing_key_variable_fails_naming_it_and_sends_nothing() {
    let transport = Arc::new(ScriptedLinear::new(|_| panic!("nothing is sent")));
    let connected = Arc::new(Mutex::new(Vec::new()));
    let survey = source(transport.clone(), connected.clone()).survey(&LinearSurveyRequest {
        workflows: vec![LinearWorkflow {
            api_key_env: "DEN_UNSET_LINEAR_KEY".to_owned(),
            ..workflow()
        }],
    });
    match state(&survey, LinearSourceKind::Viewer) {
        SourceState::Failed { error } => assert!(error.contains("DEN_UNSET_LINEAR_KEY"), "{error}"),
        other => panic!("failed, not {other:?}"),
    }
    assert!(connected.lock().unwrap().is_empty());
    assert!(transport.requests().is_empty());
}

#[test]
fn no_workflows_means_no_request_and_no_key() {
    let transport = Arc::new(ScriptedLinear::new(|_| panic!("nothing is sent")));
    let connected = Arc::new(Mutex::new(Vec::new()));
    let survey = source(transport.clone(), connected.clone()).survey(&LinearSurveyRequest {
        workflows: Vec::new(),
    });
    assert_eq!(survey, LinearSurvey::default());
    assert!(connected.lock().unwrap().is_empty());
    assert!(transport.requests().is_empty());
}

#[test]
fn each_workflow_reads_its_own_key_and_records_its_own_owner() {
    let transport = Arc::new(ScriptedLinear::new(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        Ok(ok(issues_page(Vec::new(), None)))
    }));
    let keys = Arc::new(Mutex::new(Vec::new()));
    let keys_seen = keys.clone();
    let transport_for = transport.clone();
    let source = LinearApi::with(
        Box::new(|name| match name {
            "KEY_ONE" => Some(OsString::from("one")),
            "KEY_TWO" => Some(OsString::from("two")),
            _ => None,
        }),
        Box::new(move |key: ApiKey| {
            keys_seen.lock().unwrap().push(key.expose().to_owned());
            Box::new(SharedLinear(transport_for.clone()))
        }),
        Arc::new(FixedClock(now())),
    );
    let survey = source.survey(&LinearSurveyRequest {
        workflows: vec![
            LinearWorkflow {
                api_key_env: "KEY_ONE".to_owned(),
                ..linear_workflow("acme", "acme")
            },
            LinearWorkflow {
                api_key_env: "KEY_TWO".to_owned(),
                ..linear_workflow("other", "acme")
            },
        ],
    });
    assert_eq!(*keys.lock().unwrap(), ["one", "two"]);
    let owners: Vec<&str> = survey
        .part
        .viewers
        .iter()
        .map(|v| v.owner.as_str())
        .collect();
    assert_eq!(owners, ["acme", "other"]);
    assert!(survey.coverage.is_complete());
    assert_eq!(survey.coverage.entries().len(), 4);
}

#[test]
fn reset_time_is_never_invented() {
    let (survey, _) = survey_with(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        Ok(reply(
            429,
            json!({ "errors": [{ "message": "slow down" }] }),
            &[],
        ))
    });
    assert_eq!(
        state(&survey, LinearSourceKind::AssignedIssues),
        SourceState::Partial(Incompleteness::LinearRateLimited { resets_at: None })
    );
}
