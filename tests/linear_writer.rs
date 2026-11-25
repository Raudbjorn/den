//! Adapter checks for den's two Linear writes (#38, ADR-0004) and the re-read
//! before them, through a scripted transport: nothing reaches Linear.
//!
//! The writer can express only the two operations: its port has two methods,
//! the only GraphQL `mutation`s in den are its two documents, each input
//! carries one field, and assigning names no user (the adapter reads the
//! key's own user first). Outcomes: done with what Linear returned; failed
//! when Linear answered without the change or nothing was sent; unknown when
//! the answer was lost (timeout, network, server error, unreadable reply).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{
        LinearIssueId,
        facts::{LinearState, LinearStateKind, LinearUser},
    },
    linear::{ApiKey, HttpReply, LinearApi, LinearRequest, TransportError, write::LinearWrites},
    ports::{LinearIssueRead, LinearIssueRequest, LinearSource, LinearWriteOutcome, LinearWriter},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use support::{
    build::now,
    fakes::{FixedClock, ScriptedLinear, SharedLinear},
    harness::linear_workflow,
    linear_replies::{KEY, KEY_ENV, is_viewer, ok, reply, viewer_body},
};

/// A scripted transport's answer, named for a table of cases.
type Answer = fn() -> Result<HttpReply, TransportError>;

/// The only `mutation` documents den may send.
const WRITE_OPERATIONS: [&str; 2] = ["LinearAssignToViewer", "LinearMoveState"];

fn request() -> LinearIssueRequest {
    LinearIssueRequest {
        workflow: linear_workflow("acme", "acme"),
        issue: LinearIssueId::new("acme", "id-8"),
        identifier: "TEAM-8".to_owned(),
    }
}

fn in_progress() -> LinearState {
    LinearState {
        id: "state-started".to_owned(),
        name: "In Progress".to_owned(),
        kind: LinearStateKind::Started,
    }
}

fn connect(transport: &Arc<ScriptedLinear>) -> Box<den::linear::Connect> {
    let transport = transport.clone();
    Box::new(move |key: ApiKey| {
        assert_eq!(key.expose(), KEY);
        Box::new(SharedLinear(transport.clone()))
    })
}

fn env(key: Option<&'static str>) -> Box<den::linear::Env> {
    Box::new(move |name| (name == KEY_ENV).then(|| key.map(OsString::from)).flatten())
}

fn writer(transport: &Arc<ScriptedLinear>) -> LinearWrites {
    LinearWrites::with(
        env(Some(KEY)),
        connect(transport),
        Arc::new(FixedClock(now())),
    )
}

fn scripted(
    script: impl Fn(&LinearRequest) -> Result<HttpReply, TransportError> + Send + Sync + 'static,
) -> Arc<ScriptedLinear> {
    Arc::new(ScriptedLinear::new(script))
}

/// Linear's answer to a write that it made.
fn written(assignee: Option<(&str, &str)>, state: &LinearState) -> Value {
    json!({ "data": { "issueUpdate": {
        "success": true,
        "issue": {
            "assignee": assignee.map(|(id, name)| json!({ "id": id, "name": name })),
            "state": { "id": state.id, "name": state.name, "type": "started" }
        }
    }}})
}

/// Answers the viewer query, then every write with `answer`.
fn answering(
    answer: impl Fn() -> Result<HttpReply, TransportError> + Send + Sync + 'static,
) -> Arc<ScriptedLinear> {
    scripted(move |request| {
        if is_viewer(request) {
            Ok(ok(viewer_body("acme")))
        } else {
            answer()
        }
    })
}

fn mutations(transport: &ScriptedLinear) -> Vec<LinearRequest> {
    transport
        .requests()
        .into_iter()
        .filter(|r| r.query.trim_start().starts_with("mutation") || r.query.contains("\nmutation"))
        .collect()
}

/// The keys of a request's variables.
fn variable_keys(request: &LinearRequest) -> BTreeSet<String> {
    request
        .variables
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// Every `.graphql` file under `dir`, recursively.
fn graphql_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            graphql_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "graphql") {
            found.push(path);
        }
    }
}

#[test]
fn the_writer_can_express_only_its_two_operations() {
    // Every GraphQL document in den: only the writer's two are mutations,
    // and each sets exactly one input field.
    let mut files = Vec::new();
    graphql_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 5, "{files:?}");
    let mut mutation_files: Vec<String> = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        if code.contains("mutation") {
            mutation_files.push(file.file_name().unwrap().to_string_lossy().into_owned());
            assert_eq!(code.matches("issueUpdate(").count(), 1, "{file:?}");
            let input = code.split("input: {").nth(1).unwrap();
            let fields = input.split('}').next().unwrap();
            assert_eq!(
                fields.matches(':').count(),
                1,
                "one input field in {file:?}"
            );
        }
    }
    mutation_files.sort();
    assert_eq!(
        mutation_files,
        ["assign_to_viewer.graphql", "move_state.graphql"]
    );

    // What the writer sends: a read of the key's own user, then the assign
    // naming exactly that user; the move naming exactly the state.
    let transport = answering(|| Ok(ok(written(Some(("user-me", "Me Myself")), &in_progress()))));
    let writer = writer(&transport);
    writer.assign_to_viewer(&request());
    writer.move_state(&request(), &in_progress());
    let sent = transport.requests();
    let operations: Vec<&str> = sent.iter().map(|r| r.operation.as_str()).collect();
    assert_eq!(
        operations,
        ["LinearViewer", "LinearAssignToViewer", "LinearMoveState"]
    );
    let writes = mutations(&transport);
    assert_eq!(
        writes
            .iter()
            .map(|r| r.operation.as_str())
            .collect::<Vec<_>>(),
        WRITE_OPERATIONS
    );
    assert_eq!(
        variable_keys(&writes[0]),
        BTreeSet::from(["id".to_owned(), "assigneeId".to_owned()])
    );
    assert_eq!(writes[0].variables["id"], "id-8");
    assert_eq!(writes[0].variables["assigneeId"], "user-me");
    assert_eq!(
        variable_keys(&writes[1]),
        BTreeSet::from(["id".to_owned(), "stateId".to_owned()])
    );
    assert_eq!(writes[1].variables["stateId"], "state-started");
}

#[test]
fn a_write_linear_made_is_done_with_what_linear_returned() {
    let transport = answering(|| {
        Ok(ok(written(
            Some(("user-other", "Someone Else")),
            &in_progress(),
        )))
    });
    assert_eq!(
        writer(&transport).assign_to_viewer(&request()),
        LinearWriteOutcome::Done {
            assignee: Some(LinearUser {
                id: "user-other".to_owned(),
                name: "Someone Else".to_owned(),
            }),
            state: in_progress(),
        }
    );
}

#[test]
fn a_lost_answer_is_unknown() {
    let lost: [(&str, Answer); 5] = [
        ("timeout", || {
            Err(TransportError::TimedOut {
                after: Duration::from_secs(20),
            })
        }),
        ("network", || {
            Err(TransportError::Network {
                detail: "connection reset".to_owned(),
            })
        }),
        ("server error", || Ok(reply(502, json!({}), &[]))),
        ("unreadable", || {
            Ok(HttpReply {
                status: 200,
                headers: Vec::new(),
                body: b"<html>".to_vec(),
            })
        }),
        ("done without the issue", || {
            Ok(ok(
                json!({ "data": { "issueUpdate": { "success": true, "issue": null } } }),
            ))
        }),
    ];
    for (what, answer) in lost {
        let transport = answering(answer);
        let writer = writer(&transport);
        for outcome in [
            writer.assign_to_viewer(&request()),
            writer.move_state(&request(), &in_progress()),
        ] {
            assert!(
                matches!(outcome, LinearWriteOutcome::Unknown { .. }),
                "{what}: {outcome:?}"
            );
        }
        // One request per write: never retried.
        assert_eq!(mutations(&transport).len(), 2, "{what}");
    }
}

#[test]
fn a_write_linear_refused_is_failed() {
    let refused: [(&str, Answer); 3] = [
        ("errors without data", || {
            Ok(ok(
                json!({ "errors": [{ "message": "Entity not found" }], "data": null }),
            ))
        }),
        ("success false", || {
            Ok(ok(
                json!({ "data": { "issueUpdate": { "success": false, "issue": null } } }),
            ))
        }),
        ("key refused", || {
            Ok(reply(
                401,
                json!({ "errors": [{ "message": "Authentication required" }] }),
                &[],
            ))
        }),
    ];
    for (what, answer) in refused {
        let transport = answering(answer);
        let outcome = writer(&transport).move_state(&request(), &in_progress());
        // Revoked credentials now disable the Repo connection; other refusals leave it active.
        let expected = if what == "key refused" {
            matches!(outcome, LinearWriteOutcome::Unauthenticated { .. })
        } else {
            matches!(outcome, LinearWriteOutcome::Failed { .. })
        };
        assert!(expected, "{what}: {outcome:?}");
    }
}

#[test]
fn without_the_viewer_or_a_key_nothing_is_written() {
    // The key's user can't be read: no assign is sent.
    let transport = scripted(|_| {
        Err(TransportError::Network {
            detail: "no route".to_owned(),
        })
    });
    let outcome = writer(&transport).assign_to_viewer(&request());
    assert!(
        matches!(outcome, LinearWriteOutcome::Failed { .. }),
        "{outcome:?}"
    );
    assert!(mutations(&transport).is_empty());

    // A key for another workspace: no assign is sent.
    let transport = scripted(|request| {
        assert!(is_viewer(request), "only the viewer is read");
        Ok(ok(viewer_body("other")))
    });
    let outcome = writer(&transport).assign_to_viewer(&request());
    // A different account/workspace now revokes the binding, rather than reporting a transient failure.
    let LinearWriteOutcome::Unauthenticated { error } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(error.contains("\"other\""), "{error}");
    assert!(mutations(&transport).is_empty());

    // No key: nothing at all is sent.
    let transport = scripted(|_| panic!("nothing is sent without a key"));
    let writer = LinearWrites::with(env(None), connect(&transport), Arc::new(FixedClock(now())));
    let outcome = writer.move_state(&request(), &in_progress());
    // Missing credentials are distinguished from a transient read failure so App can deactivate.
    let LinearWriteOutcome::Unauthenticated { error } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(error.contains(KEY_ENV) && !error.contains(KEY), "{error}");
}

fn source(transport: &Arc<ScriptedLinear>) -> LinearApi {
    LinearApi::with(
        env(Some(KEY)),
        connect(transport),
        Arc::new(FixedClock(now())),
    )
}

fn issue_now(workspace: &str, more_states: bool) -> Value {
    json!({ "data": {
        "organization": { "urlKey": workspace },
        "issue": {
            "id": "id-8",
            "assignee": null,
            "state": { "id": "state-todo", "name": "Todo", "type": "unstarted" },
            "team": { "id": "team-ops", "states": {
                "nodes": [
                    { "id": "state-todo", "name": "Todo", "type": "unstarted" },
                    { "id": "state-started", "name": "In Progress", "type": "started" }
                ],
                "pageInfo": { "hasNextPage": more_states }
            }}
        }
    }})
}

#[test]
fn a_reread_reads_the_issue_and_its_own_teams_states() {
    let transport = scripted(|_| Ok(ok(issue_now("acme", false))));
    let read = source(&transport).read_issue(&request());
    let LinearIssueRead::Read(now) = read else {
        panic!("{read:?}");
    };
    assert_eq!(now.assignee, None);
    assert_eq!(now.state.name, "Todo");
    assert_eq!(now.team_states.len(), 2);
    assert_eq!(now.team_states[1], in_progress());
    assert!(now.states_complete);
    let sent = transport.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].operation, "LinearIssueNow");
    assert_eq!(sent[0].variables["id"], "id-8");
    assert!(!sent[0].query.contains("mutation"));

    let transport = scripted(|_| Ok(ok(issue_now("acme", true))));
    let LinearIssueRead::Read(now) = source(&transport).read_issue(&request()) else {
        panic!("read");
    };
    assert!(!now.states_complete);
}

#[test]
fn a_reread_den_cant_trust_fails() {
    let untrusted: [(&str, Answer); 4] = [
        ("another workspace", || Ok(ok(issue_now("other", false)))),
        ("no such issue", || {
            Ok(ok(
                json!({ "data": { "organization": { "urlKey": "acme" }, "issue": null } }),
            ))
        }),
        ("errors next to the issue", || {
            let mut body = issue_now("acme", false);
            body["errors"] = json!([{ "message": "partial" }]);
            Ok(ok(body))
        }),
        ("timeout", || {
            Err(TransportError::TimedOut {
                after: Duration::from_secs(20),
            })
        }),
    ];
    for (what, answer) in untrusted {
        let transport = scripted(move |_| answer());
        let read = source(&transport).read_issue(&request());
        assert!(
            matches!(read, LinearIssueRead::Failed { .. }),
            "{what}: {read:?}"
        );
    }
}
