//! Scripted Linear replies for the Linear source's adapter checks: the
//! viewer, pages of assigned issues, and a source wired to a scripted
//! transport with a known key.

use super::{
    build::now,
    fakes::{FixedClock, ScriptedLinear, SharedLinear},
};
use den::{
    linear::{ApiKey, HttpReply, LinearApi, LinearRequest},
    ports::Clock,
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    sync::{Arc, Mutex},
};

/// A key that would be easy to spot anywhere it leaked.
pub const KEY: &str = "lin_api_SECRET_do_not_log_0123456789";
pub const KEY_ENV: &str = "DEN_TEST_LINEAR_KEY";

pub fn ok(body: Value) -> HttpReply {
    reply(200, body, &[])
}

pub fn reply(status: u16, body: Value, headers: &[(&str, &str)]) -> HttpReply {
    HttpReply {
        status,
        headers: headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
        body: serde_json::to_vec(&body).unwrap(),
    }
}

pub fn viewer_body(workspace: &str) -> Value {
    json!({ "data": {
        "viewer": { "id": "user-me", "name": "Me Myself" },
        "organization": { "urlKey": workspace }
    }})
}

pub fn issue_node(id: &str, identifier: &str) -> Value {
    json!({
        "id": id,
        "identifier": identifier,
        "title": format!("Do {identifier}"),
        "url": format!("https://linear.app/acme/issue/{}", identifier.to_lowercase()),
        "priority": 2.0,
        "createdAt": "2026-11-01T10:00:00.000Z",
        "updatedAt": "2026-11-18T10:00:00.000Z",
        "branchName": format!("me/{}-do", identifier.to_lowercase()),
        "team": { "id": "team-1", "key": "TEAM" },
        "state": { "id": "state-1", "name": "In Progress", "type": "started" },
        "assignee": { "id": "user-me", "name": "Me Myself" },
        "cycle": { "id": "cycle-3", "number": 3.0, "name": null, "isActive": true }
    })
}

pub fn issues_page(nodes: Vec<Value>, next: Option<&str>) -> Value {
    json!({ "data": { "viewer": { "assignedIssues": {
        "nodes": nodes,
        "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next }
    }}}})
}

pub fn is_viewer(request: &LinearRequest) -> bool {
    request.operation == "LinearViewer"
}

/// A Linear source whose env has [`KEY`] under [`KEY_ENV`], whose transport
/// is `transport`, and which records every key it was asked to connect with.
pub fn source(transport: Arc<ScriptedLinear>, keys: Arc<Mutex<Vec<String>>>) -> LinearApi {
    source_with_clock(transport, keys, Arc::new(FixedClock(now())))
}

/// [`source`], reading the time from `clock`.
pub fn source_with_clock(
    transport: Arc<ScriptedLinear>,
    keys: Arc<Mutex<Vec<String>>>,
    clock: Arc<dyn Clock>,
) -> LinearApi {
    LinearApi::with(
        Box::new(|name| (name == KEY_ENV).then(|| OsString::from(KEY))),
        Box::new(move |key: ApiKey| {
            keys.lock().unwrap().push(key.expose().to_owned());
            Box::new(SharedLinear(transport.clone()))
        }),
        clock,
    )
}
