//! The background worker runs a Linear survey on its own lane: a Linear call
//! that hangs never holds back GitHub's answer.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Event, SurveyId},
    ports::{LinearSource, LinearSurvey, LinearSurveyRequest, Ports, SurveyRequest},
    runtime::Worker,
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
    harness::linear_workflow,
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
fn github_answers_while_a_linear_survey_hangs() {
    let (worker, release) = worker();
    let id = SurveyId(0);
    // In the App's order: Linear first, then GitHub.
    worker
        .submit(Effect::SurveyLinear {
            id,
            request: LinearSurveyRequest {
                workflows: vec![linear_workflow("acme", "acme")],
            },
        })
        .expect("queued");
    worker
        .submit(Effect::Survey {
            id,
            request: SurveyRequest {
                owners: vec!["acme".to_owned()],
                upstreams: Default::default(),
            },
        })
        .expect("queued");

    let first = next_event(&worker).expect("GitHub answers while Linear is held");
    assert!(
        matches!(first, Event::SurveyFinished { outcome: Ok(_), .. }),
        "{first:?}"
    );
    release.send(()).expect("Linear still waiting");
    let second = next_event(&worker).expect("Linear answers once let go");
    assert!(
        matches!(second, Event::LinearSurveyFinished { .. }),
        "{second:?}"
    );
}
