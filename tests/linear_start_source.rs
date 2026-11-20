//! Adapter checks for what Start reads from Linear (#33), through a scripted
//! transport: each team's workflow states and the start teams' unassigned
//! issues, each with its own coverage; the filter den sends; and a team or
//! state name the config gives that Linear doesn't have, which fails the
//! team-states source naming it.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    config::LinearWorkflow,
    domain::{
        Incompleteness, LinearIssueId, LinearSourceKind, Source, SourceKind, SourceState,
        facts::LinearStateKind,
    },
    linear::{HttpReply, LinearRequest, MAX_LINEAR_PAGES, TransportError},
    ports::{LinearSource, LinearSurvey, LinearSurveyRequest},
};
use serde_json::{Value, json};
use std::sync::Arc;
use support::{
    fakes::ScriptedLinear,
    harness::linear_workflow,
    linear_replies::{issue_node, issues_page, ok, source, viewer_body},
};

/// `acme`'s workflow with start team `TEAM` and the two state names.
fn start_workflow() -> LinearWorkflow {
    LinearWorkflow {
        start_teams: vec!["TEAM".to_owned()],
        in_progress_state: Some("In Progress".to_owned()),
        release_state: Some("Ready for Release".to_owned()),
        ..linear_workflow("acme", "acme")
    }
}

fn state_node(id: &str, name: &str, kind: &str) -> Value {
    json!({ "id": id, "name": name, "type": kind })
}

/// Team `key`'s node with the usual states.
fn team_node(key: &str) -> Value {
    team_node_with(
        key,
        vec![
            state_node(&format!("{key}-backlog"), "Backlog", "backlog"),
            state_node(&format!("{key}-todo"), "Todo", "unstarted"),
            state_node(&format!("{key}-doing"), "In Progress", "started"),
            state_node(&format!("{key}-release"), "Ready for Release", "started"),
            state_node(&format!("{key}-done"), "Done", "completed"),
        ],
    )
}

fn team_node_with(key: &str, states: Vec<Value>) -> Value {
    json!({
        "id": format!("team-{}", key.to_lowercase()),
        "key": key,
        "name": format!("Team {key}"),
        "states": { "nodes": states, "pageInfo": { "hasNextPage": false } }
    })
}

fn teams_page(nodes: Vec<Value>, next: Option<&str>) -> Value {
    json!({ "data": { "teams": {
        "nodes": nodes,
        "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next }
    }}})
}

/// An unassigned, unstarted issue of team `TEAM`.
fn start_node(id: &str, identifier: &str) -> Value {
    let mut node = issue_node(id, identifier);
    node["assignee"] = Value::Null;
    node["state"] = state_node("TEAM-todo", "Todo", "unstarted");
    node
}

fn start_page(nodes: Vec<Value>, next: Option<&str>) -> Value {
    json!({ "data": { "issues": {
        "nodes": nodes,
        "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next }
    }}})
}

type Script = dyn Fn(&LinearRequest) -> Result<HttpReply, TransportError> + Send + Sync;

/// Answers each operation by name; `teams` and `start` answer their own.
fn script(
    teams: impl Fn(&LinearRequest) -> HttpReply + Send + Sync + 'static,
    start: impl Fn(&LinearRequest) -> HttpReply + Send + Sync + 'static,
) -> Box<Script> {
    Box::new(move |request| {
        Ok(match request.operation.as_str() {
            "LinearViewer" => ok(viewer_body("acme")),
            "LinearAssignedIssues" => ok(issues_page(vec![issue_node("id-1", "TEAM-1")], None)),
            "LinearTeamStates" => teams(request),
            "LinearStartTeamIssues" => start(request),
            other => panic!("unexpected operation {other}"),
        })
    })
}

fn run_survey(
    workflow: LinearWorkflow,
    script: Box<Script>,
) -> (LinearSurvey, Arc<ScriptedLinear>) {
    let transport = Arc::new(ScriptedLinear::new(script));
    let survey = source(transport.clone(), Arc::default()).survey(&LinearSurveyRequest {
        workflows: vec![workflow],
    });
    (survey, transport)
}

fn state(survey: &LinearSurvey, kind: LinearSourceKind) -> Option<SourceState> {
    survey
        .coverage
        .get(&Source::Owner {
            owner: "acme".to_owned(),
            kind: SourceKind::Linear(kind),
        })
        .cloned()
}

fn failure(survey: &LinearSurvey, kind: LinearSourceKind) -> String {
    match state(survey, kind) {
        Some(SourceState::Failed { error }) => error,
        other => panic!("{kind:?} failed, not {other:?}"),
    }
}

fn operations(transport: &ScriptedLinear) -> Vec<String> {
    transport
        .requests()
        .into_iter()
        .map(|r| r.operation)
        .collect()
}

fn all_teams(_: &LinearRequest) -> HttpReply {
    ok(teams_page(vec![team_node("TEAM"), team_node("OPS")], None))
}

fn two_starts(_: &LinearRequest) -> HttpReply {
    ok(start_page(
        vec![start_node("id-8", "TEAM-8"), start_node("id-9", "TEAM-9")],
        None,
    ))
}

#[test]
fn reads_team_states_and_the_start_teams_unassigned_issues() {
    let (survey, transport) = run_survey(start_workflow(), script(all_teams, two_starts));

    assert_eq!(
        operations(&transport),
        [
            "LinearViewer",
            "LinearAssignedIssues",
            "LinearTeamStates",
            "LinearStartTeamIssues"
        ]
    );
    assert!(
        transport
            .requests()
            .iter()
            .all(|r| r.query.trim_start().starts_with("query")),
        "read-only: queries only"
    );
    for kind in [
        LinearSourceKind::Viewer,
        LinearSourceKind::AssignedIssues,
        LinearSourceKind::TeamStates,
        LinearSourceKind::StartTeamIssues,
    ] {
        assert_eq!(
            state(&survey, kind),
            Some(SourceState::Complete),
            "{kind:?}"
        );
    }

    let team = survey
        .part
        .team_states
        .iter()
        .find(|t| t.team.key == "TEAM")
        .expect("TEAM's states");
    assert_eq!(
        (team.owner.as_str(), team.workspace.as_str()),
        ("acme", "acme")
    );
    assert_eq!(team.team.id, "team-team");
    assert_eq!(team.states.len(), 5);
    assert_eq!(team.states[1].kind, LinearStateKind::Unstarted);
    assert_eq!(survey.part.team_states.len(), 2);

    let identifiers: Vec<&str> = survey
        .part
        .issues
        .iter()
        .map(|i| i.identifier.as_str())
        .collect();
    assert_eq!(identifiers, ["TEAM-1", "TEAM-8", "TEAM-9"]);
    let start = &survey.part.issues[1];
    assert_eq!(start.id, LinearIssueId::new("acme", "id-8"));
    assert_eq!(start.assignee, None);
    assert_eq!(start.state.kind, LinearStateKind::Unstarted);
}

/// What den asks Linear for: unassigned, the start teams by key (any case),
/// unstarted, plus backlog and the active cycle only when the workflow says.
#[test]
fn the_start_filter_follows_the_workflow() {
    let filter = |workflow: LinearWorkflow| {
        let (_, transport) = run_survey(workflow, script(all_teams, two_starts));
        transport
            .requests()
            .into_iter()
            .find(|r| r.operation == "LinearStartTeamIssues")
            .expect("asked")
            .variables["filter"]
            .clone()
    };

    let plain = filter(LinearWorkflow {
        start_teams: vec!["TEAM".to_owned(), "web".to_owned()],
        ..start_workflow()
    });
    assert_eq!(
        plain,
        json!({
            "team": { "or": [
                { "key": { "eqIgnoreCase": "TEAM" } },
                { "key": { "eqIgnoreCase": "web" } }
            ] },
            "assignee": { "null": true },
            "state": { "type": { "in": ["unstarted"] } }
        })
    );

    let wider = filter(LinearWorkflow {
        include_backlog: true,
        active_cycle_only: true,
        ..start_workflow()
    });
    assert_eq!(
        wider["state"],
        json!({ "type": { "in": ["unstarted", "backlog"] } })
    );
    assert_eq!(wider["cycle"], json!({ "isActive": { "eq": true } }));
}

#[test]
fn a_start_team_linear_lacks_fails_team_states_naming_it() {
    let (survey, _) = run_survey(
        LinearWorkflow {
            start_teams: vec!["TEAM".to_owned(), "NOPE".to_owned()],
            ..start_workflow()
        },
        script(all_teams, two_starts),
    );
    let error = failure(&survey, LinearSourceKind::TeamStates);
    assert!(
        error.contains("\"NOPE\"") && error.contains("start_teams") && error.contains("acme"),
        "{error}"
    );
    assert!(!error.contains("\"TEAM\""), "TEAM exists: {error}");
    // The rule isn't silently off: TEAM's issues are still offered, and the
    // failure is the team-states source's alone.
    assert_eq!(
        state(&survey, LinearSourceKind::StartTeamIssues),
        Some(SourceState::Complete)
    );
    assert_eq!(survey.part.issues.len(), 3);
    assert!(
        !survey.part.team_states.is_empty(),
        "what Linear has is kept"
    );
}

#[test]
fn a_state_name_a_start_team_lacks_fails_team_states_naming_it() {
    let (survey, _) = run_survey(
        LinearWorkflow {
            in_progress_state: Some("In progress".to_owned()),
            release_state: Some("Shipped".to_owned()),
            ..start_workflow()
        },
        script(all_teams, two_starts),
    );
    let error = failure(&survey, LinearSourceKind::TeamStates);
    for expected in [
        "in_progress_state \"In progress\" is not a state of start team TEAM",
        "Linear spells it \"In Progress\"",
        "release_state \"Shipped\" is not a state of start team TEAM",
    ] {
        assert!(error.contains(expected), "{expected:?} in {error}");
    }
    assert!(!error.contains("OPS"), "OPS is no start team: {error}");
}

/// Without start teams, a state name is checked against every team: it must
/// be some team's. Start-team issues aren't asked for or recorded.
#[test]
fn without_start_teams_a_state_name_must_be_some_teams() {
    let only_ops = |_: &LinearRequest| {
        ok(teams_page(
            vec![team_node_with(
                "OPS",
                vec![state_node("s", "Ready for Release", "started")],
            )],
            None,
        ))
    };
    let workflow = LinearWorkflow {
        start_teams: Vec::new(),
        in_progress_state: None,
        ..start_workflow()
    };
    let (survey, transport) = run_survey(workflow.clone(), script(only_ops, two_starts));
    assert_eq!(
        state(&survey, LinearSourceKind::TeamStates),
        Some(SourceState::Complete)
    );
    assert_eq!(state(&survey, LinearSourceKind::StartTeamIssues), None);
    assert!(
        !operations(&transport).contains(&"LinearStartTeamIssues".to_owned()),
        "no start team, no start-team request"
    );

    let (survey, _) = run_survey(
        LinearWorkflow {
            release_state: Some("Released".to_owned()),
            ..workflow
        },
        script(only_ops, two_starts),
    );
    let error = failure(&survey, LinearSourceKind::TeamStates);
    assert!(
        error.contains("release_state \"Released\" is not a state of any team"),
        "{error}"
    );
}

/// A workflow naming no start team and no state asks Linear only what #32
/// did, and records no team-states or start-team source.
#[test]
fn nothing_to_check_asks_nothing_more() {
    let (survey, transport) = run_survey(
        linear_workflow("acme", "acme"),
        script(all_teams, two_starts),
    );
    assert_eq!(
        operations(&transport),
        ["LinearViewer", "LinearAssignedIssues"]
    );
    assert_eq!(state(&survey, LinearSourceKind::TeamStates), None);
    assert_eq!(state(&survey, LinearSourceKind::StartTeamIssues), None);
}

/// A name den couldn't check is never called missing: a team list cut off at
/// the page limit, or a team whose states run past one page, is partial.
#[test]
fn an_unread_team_or_state_is_partial_not_missing() {
    let endless = |request: &LinearRequest| {
        let page = request.variables["after"]
            .as_str()
            .map_or(0, |c| c.parse::<u32>().unwrap());
        ok(teams_page(
            vec![team_node(&format!("T{page}"))],
            Some(&(page + 1).to_string()),
        ))
    };
    let (survey, _) = run_survey(start_workflow(), script(endless, two_starts));
    match state(&survey, LinearSourceKind::TeamStates) {
        Some(SourceState::Partial(Incompleteness::PaginationStopped { pages_read, .. })) => {
            assert_eq!(pages_read, MAX_LINEAR_PAGES);
        }
        other => panic!("partial at the page limit, not {other:?}"),
    }

    let long_states = |_: &LinearRequest| {
        let mut team = team_node_with("TEAM", vec![state_node("s", "Todo", "unstarted")]);
        team["states"]["pageInfo"]["hasNextPage"] = json!(true);
        ok(teams_page(vec![team], None))
    };
    let (survey, _) = run_survey(start_workflow(), script(long_states, two_starts));
    match state(&survey, LinearSourceKind::TeamStates) {
        Some(SourceState::Partial(Incompleteness::FollowUpTruncated { what })) => {
            assert!(what.contains("TEAM"), "{what}");
        }
        other => panic!("partial for TEAM's unread states, not {other:?}"),
    }
}

/// Start-team issues and team states each have their own coverage: trouble
/// in one leaves the other as it was read.
#[test]
fn start_team_issues_and_team_states_have_their_own_coverage() {
    let errors_with_data = |_: &LinearRequest| {
        let mut body = start_page(vec![start_node("id-8", "TEAM-8")], None);
        body["errors"] = json!([{ "message": "one issue hidden" }]);
        ok(body)
    };
    let (survey, _) = run_survey(start_workflow(), script(all_teams, errors_with_data));
    assert_eq!(
        state(&survey, LinearSourceKind::TeamStates),
        Some(SourceState::Complete)
    );
    assert!(
        matches!(
            state(&survey, LinearSourceKind::StartTeamIssues),
            Some(SourceState::Partial(
                Incompleteness::LinearErrorsAlongsideData { .. }
            ))
        ),
        "{:?}",
        survey.coverage
    );

    let broken_teams =
        |_: &LinearRequest| ok(json!({ "data": null, "errors": [{ "message": "boom" }] }));
    let (survey, _) = run_survey(start_workflow(), script(broken_teams, two_starts));
    assert!(matches!(
        state(&survey, LinearSourceKind::TeamStates),
        Some(SourceState::Failed { .. })
    ));
    assert_eq!(
        state(&survey, LinearSourceKind::StartTeamIssues),
        Some(SourceState::Complete)
    );
}

/// No key, or no viewer: every source the workflow reads is failed, so none
/// of the four reads as complete or as never asked.
#[test]
fn a_missing_key_or_refused_viewer_fails_all_four_sources() {
    let transport = Arc::new(ScriptedLinear::new(|_| panic!("nothing is sent")));
    let survey = den::linear::LinearApi::with(
        Box::new(|_| None),
        Box::new(move |_| Box::new(support::fakes::SharedLinear(transport.clone()))),
        Arc::new(support::fakes::FixedClock(support::build::now())),
    )
    .survey(&LinearSurveyRequest {
        workflows: vec![start_workflow()],
    });
    for kind in [
        LinearSourceKind::Viewer,
        LinearSourceKind::AssignedIssues,
        LinearSourceKind::TeamStates,
        LinearSourceKind::StartTeamIssues,
    ] {
        assert!(
            failure(&survey, kind).contains("DEN_TEST_LINEAR_KEY"),
            "{kind:?}"
        );
    }

    let refused = Box::new(|request: &LinearRequest| {
        assert_eq!(
            request.operation, "LinearViewer",
            "nothing after the viewer"
        );
        Ok(support::linear_replies::reply(
            401,
            json!({ "errors": [{ "message": "Authentication required" }] }),
            &[],
        ))
    });
    let (survey, _) = run_survey(start_workflow(), refused);
    for kind in [
        LinearSourceKind::TeamStates,
        LinearSourceKind::StartTeamIssues,
    ] {
        assert!(
            failure(&survey, kind).contains("DEN_TEST_LINEAR_KEY"),
            "{kind:?}"
        );
    }
}

/// An issue Linear returns twice (or that is also assigned to me) is kept once.
#[test]
fn an_issue_is_kept_once() {
    let repeated = |_: &LinearRequest| {
        ok(start_page(
            vec![
                start_node("id-1", "TEAM-1"),
                start_node("id-8", "TEAM-8"),
                start_node("id-8", "TEAM-8"),
            ],
            None,
        ))
    };
    let (survey, _) = run_survey(start_workflow(), script(all_teams, repeated));
    let identifiers: Vec<&str> = survey
        .part
        .issues
        .iter()
        .map(|i| i.identifier.as_str())
        .collect();
    assert_eq!(identifiers, ["TEAM-1", "TEAM-8"]);
}
