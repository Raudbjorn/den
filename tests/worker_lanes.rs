//! The background worker runs a Linear survey on its own lane: a Linear call
//! that hangs never holds back GitHub's answer.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{App, Effect, Event, SurveyId},
    ports::{LinearSource, LinearSurvey, LinearSurveyRequest, Ports, SurveyRequest},
    runtime::{Flow, Worker},
};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};
use support::{
    build::{ObservationBuilder, now},
    fakes::{
        FakeCache, FakeGitHub, FakeJobs, FakeLauncher, FakeUpstreams, FakeWorkspace, FixedClock,
    },
    harness::{config_with_linear, linear_workflow},
};

/// How long the test waits for an answer the worker should give at once.
const ANSWER_WAIT: Duration = Duration::from_secs(10);

/// A Linear source that doesn't answer until the test lets it.
struct HeldLinear {
    release: Mutex<Receiver<()>>,
}

impl LinearSource for HeldLinear {
    fn survey(&self, _request: &LinearSurveyRequest) -> LinearSurvey {
        // A dropped sender also lets it go, so the thread never outlives the test.
        let _ = self.release.lock().expect("lock").recv();
        LinearSurvey::default()
    }
}

fn worker() -> (Worker, Sender<()>) {
    let (release, held) = mpsc::channel();
    let github = Arc::new(FakeGitHub::default());
    github.push_answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .build()));
    let ports = Ports {
        github,
        linear: Arc::new(HeldLinear {
            release: Mutex::new(held),
        }),
        cache: Arc::new(FakeCache::default()),
        workspace: Arc::new(FakeWorkspace::default()),
        launcher: Arc::new(FakeLauncher::default()),
        jobs: Arc::new(FakeJobs::default()),
        upstreams: Arc::new(FakeUpstreams::default()),
        clock: Arc::new(FixedClock(now())),
    };
    (Worker::start(ports), release)
}

/// Waits for the next Event the worker finishes.
fn next_event(worker: &Worker) -> Option<Event> {
    let deadline = Instant::now() + ANSWER_WAIT;
    while Instant::now() < deadline {
        if let Some(event) = worker.finished().next() {
            return Some(event);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

#[test]
fn github_and_local_writes_answer_while_a_linear_survey_hangs() {
    let (worker, release) = worker();
    let id = SurveyId(0);
    let mut app = App::new(Ok(config_with_linear(&["acme"], &[("acme", "acme")])));
    let mut view = app.view();
    // In the App's order: Linear first, then GitHub.
    let effects = vec![
        Effect::SurveyLinear {
            id,
            request: LinearSurveyRequest {
                workflows: vec![linear_workflow("acme", "acme")],
            },
        },
        Effect::Survey {
            id,
            request: SurveyRequest {
                owners: vec!["acme".to_owned()],
                upstreams: Default::default(),
            },
        },
        Effect::StoreUpstreams {
            upstreams: Default::default(),
        },
    ];
    // The merged dispatcher keeps local writes and GitHub independent of Linear.
    assert_eq!(
        worker.dispatch(&mut app, &mut view, effects),
        Flow::Continue
    );

    let finished = [
        next_event(&worker).expect("GitHub or the store answers while Linear is held"),
        next_event(&worker).expect("both answer while Linear is held"),
    ];
    assert!(
        finished
            .iter()
            .any(|e| matches!(e, Event::SurveyFinished { outcome: Ok(_), .. })),
        "{finished:?}"
    );
    assert!(
        finished
            .iter()
            .any(|e| matches!(e, Event::UpstreamsStored { outcome: Ok(()) })),
        "{finished:?}"
    );
    release.send(()).expect("Linear still waiting");
    let second = next_event(&worker).expect("Linear answers once let go");
    assert!(
        matches!(second, Event::LinearSurveyFinished { .. }),
        "{second:?}"
    );
}
