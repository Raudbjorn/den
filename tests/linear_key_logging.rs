//! The Linear API key never reaches den's log (#32): its own test binary,
//! because `tracing` caches whether a call site is enabled process-wide, so a
//! thread-local subscriber can miss events while other tests run in parallel
//! threads of the same binary.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    linear::{ApiKey, HttpsTransport, TransportError},
    ports::{LinearSource, LinearSurveyRequest},
};
use std::{
    io,
    sync::{Arc, Mutex},
};
use support::{
    fakes::ScriptedLinear,
    harness::linear_workflow,
    linear_replies::{KEY, is_viewer, issue_node, issues_page, ok, source, viewer_body},
};

/// A `MakeWriter` collecting everything the log would write.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_key_reaches_the_transport_and_nothing_else() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let transport = Arc::new(ScriptedLinear::new(|request| {
        if is_viewer(request) {
            return Ok(ok(viewer_body("acme")));
        }
        if request.variables["after"].is_null() {
            return Ok(ok(issues_page(
                vec![issue_node("id-1", "TEAM-1")],
                Some("c"),
            )));
        }
        // A failure too, so error paths are logged as well.
        Err(TransportError::Network {
            detail: "connection reset".to_owned(),
        })
    }));
    let connected = Arc::new(Mutex::new(Vec::new()));
    let survey = tracing::subscriber::with_default(subscriber, || {
        source(transport.clone(), connected.clone()).survey(&LinearSurveyRequest {
            workflows: vec![linear_workflow("acme", "acme")],
        })
    });

    assert_eq!(
        *connected.lock().unwrap(),
        [KEY],
        "the transport got the key"
    );
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("LinearAssignedIssues"),
        "requests were logged: {log}"
    );
    assert!(!log.contains(KEY), "the key is in the log: {log}");
    for request in transport.requests() {
        assert!(!format!("{request:?}").contains(KEY), "{request:?}");
    }
    assert!(!format!("{survey:?}").contains(KEY));
    assert_eq!(format!("{:?}", ApiKey::new(KEY)), "ApiKey([redacted])");
    assert!(!format!("{:?}", HttpsTransport::new(ApiKey::new(KEY))).contains(KEY));
}
