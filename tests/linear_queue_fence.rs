//! A Repo switch fences work already waiting on the Linear lane.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use den::{
    app::{App, Effect, Event, SurveyId},
    domain::RepoId,
    ports::{
        LinearIssueRead, LinearIssueRequest, LinearSource, LinearSurvey, LinearSurveyRequest, Ports,
    },
    runtime::Worker,
    start::Start,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use support::{
    build::now,
    fakes::*,
    harness::{config_with_linear, linear_workflow},
};
struct Held {
    started: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    calls: Arc<AtomicUsize>,
}
impl LinearSource for Held {
    fn survey(&self, _: &LinearSurveyRequest) -> LinearSurvey {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        LinearSurvey::default()
    }
    fn read_issue(&self, _: &LinearIssueRequest) -> LinearIssueRead {
        unreachable!()
    }
}
#[test]
fn switching_repos_rejects_a_queued_linear_operation_before_io() {
    let (started, start_rx) = mpsc::sync_channel(1);
    let (release, release_rx) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker = Worker::start(Ports {
        github: Arc::new(FakeGitHub::default()),
        linear: Arc::new(Held {
            started,
            release: Mutex::new(release_rx),
            calls: calls.clone(),
        }),
        linear_writer: Arc::new(FakeLinearWriter::default()),
        cache: Arc::new(FakeCache::default()),
        workspace: Arc::new(FakeWorkspace::default()),
        launcher: Arc::new(FakeLauncher::default()),
        jobs: Arc::new(FakeJobs::default()),
        upstreams: Arc::new(FakeUpstreams::default()),
        clock: Arc::new(FixedClock(now())),
    });
    let config = config_with_linear(&["acme"], &[("acme", "acme")]);
    let mut app = App::new(Ok(config.clone())).with_start(Start::Repo(RepoId::new("acme", "one")));
    let survey = |id| Effect::SurveyLinear {
        id: SurveyId(id),
        request: LinearSurveyRequest {
            workflows: vec![linear_workflow("acme", "acme")],
        },
    };
    let mut view = app.view();
    worker.dispatch(&mut app, &mut view, vec![survey(1)]);
    start_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.dispatch(&mut app, &mut view, vec![survey(2)]);
    app = App::new(Ok(config)).with_start(Start::Repo(RepoId::new("acme", "two")));
    worker.dispatch(&mut app, &mut view, vec![]);
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut rejected = false;
    while Instant::now() < deadline && !rejected {
        for event in worker.finished() {
            if let Event::EffectRejected { effect, .. } = event {
                rejected = matches!(
                    *effect,
                    Effect::SurveyLinear {
                        id: SurveyId(2),
                        ..
                    }
                );
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // In-flight I/O may finish, but the second operation must never start.
    assert!(rejected);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
