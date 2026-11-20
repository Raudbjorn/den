//! The interactive runtime's Worker: den's state files are written on their own
//! lane, never behind a survey, and quitting waits (bounded) for those writes.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{App, Effect, Event, SurveyId},
    domain::{Observation, upstream::UpstreamTargets},
    ports::{
        GitHubSource, MemberStatesRead, MemberStatesRequest, Ports, SurveyFailure, SurveyRequest,
        UpstreamStore,
    },
    runtime::{FinishError, Flow, Worker},
    upstreams::UpstreamStoreError,
};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};
use support::{
    build::{now, repo_id},
    fakes::{FakeGitHub, FakeUpstreams, ports},
    harness::config,
};

/// Long enough for any wait in these tests to succeed on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(5);

/// Blocks a port call until the test releases it, as a slow `gh` or a hung
/// disk would. Calls still blocked when a test ends find nobody listening: fine.
struct Gate {
    started: Mutex<Sender<()>>,
    release: Mutex<Receiver<()>>,
}

impl Gate {
    /// The gate, a receiver told when a call reaches it, and its release.
    fn new() -> (Self, Receiver<()>, Sender<()>) {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let gate = Self {
            started: Mutex::new(started_tx),
            release: Mutex::new(release_rx),
        };
        (gate, started_rx, release_tx)
    }

    fn pass(&self) {
        let _ = self.started.lock().unwrap().send(());
        self.release.lock().unwrap().recv().ok();
    }
}

impl GitHubSource for Gate {
    fn survey(&self, _: &SurveyRequest) -> Result<Observation, SurveyFailure> {
        self.pass();
        Err(SurveyFailure::NotLoggedIn {
            detail: "released".to_owned(),
        })
    }

    fn member_states(&self, _: &MemberStatesRequest) -> MemberStatesRead {
        MemberStatesRead::default()
    }
}

/// Upstream choices whose writes wait at a [`Gate`] before landing.
struct GatedUpstreams {
    gate: Gate,
    inner: FakeUpstreams,
}

impl UpstreamStore for GatedUpstreams {
    fn load(&self) -> Result<UpstreamTargets, UpstreamStoreError> {
        self.inner.load()
    }

    fn store(&self, upstreams: &UpstreamTargets) -> Result<(), UpstreamStoreError> {
        self.gate.pass();
        self.inner.store(upstreams)
    }
}

fn app() -> App {
    App::new(Ok(config(&["octo"])))
}

fn survey() -> Effect {
    Effect::Survey {
        id: SurveyId(0),
        request: SurveyRequest {
            owners: vec!["octo".to_owned()],
            upstreams: UpstreamTargets::default(),
        },
    }
}

fn store(upstreams: &UpstreamTargets) -> Effect {
    Effect::StoreUpstreams {
        upstreams: upstreams.clone(),
    }
}

/// Dispatches `effects` from a fresh view of `app`, as the event loop does.
fn dispatch(worker: &Worker, app: &mut App, effects: Vec<Effect>) -> Flow {
    let mut view = app.view();
    worker.dispatch(app, &mut view, effects)
}

/// A Worker whose survey is held at its gate: the remote lane is busy until
/// the returned sender releases it.
fn held_survey_worker(ports: Ports, app: &mut App) -> (Worker, Sender<()>) {
    let (gate, started, release) = Gate::new();
    let worker = Worker::start(Ports {
        github: Arc::new(gate),
        ..ports
    });
    dispatch(&worker, app, vec![survey()]);
    started
        .recv_timeout(PATIENCE)
        .expect("the survey started on the remote lane");
    (worker, release)
}

/// An Upstream choice made while a survey runs is stored before the survey
/// ends, so quitting mid-survey cannot lose it.
#[test]
fn an_upstream_choice_is_stored_while_a_survey_holds_the_remote_lane() {
    let upstreams = Arc::new(FakeUpstreams::default());
    let ports = Ports {
        upstreams: upstreams.clone(),
        ..ports(Arc::new(FakeGitHub::answering([])), now())
    };
    let mut app = app();
    let (worker, release) = held_survey_worker(ports, &mut app);

    let chosen = UpstreamTargets::new([repo_id("octo/app")]);
    assert_eq!(
        dispatch(&worker, &mut app, vec![store(&chosen)]),
        Flow::Continue
    );

    let deadline = Instant::now() + PATIENCE;
    let stored = loop {
        if let Some(event) = worker.finished().next() {
            break event;
        }
        assert!(Instant::now() < deadline, "the choice was not stored");
        std::thread::yield_now();
    };
    assert!(
        matches!(stored, Event::UpstreamsStored { outcome: Ok(()) }),
        "{stored:?}"
    );
    assert_eq!(upstreams.stored(), [chosen], "stored while the survey runs");
    release.send(()).unwrap();
}

/// A remote lane so busy it refuses work (its rejections go to the App, see
/// runtime.rs's unit tests) does not hold up den's state files.
#[test]
fn a_full_remote_lane_does_not_hold_up_writes() {
    /// More than a lane's queue holds; any bound past it will do.
    const MORE_THAN_THE_QUEUE: usize = 1024;
    let upstreams = Arc::new(FakeUpstreams::default());
    let ports = Ports {
        upstreams: upstreams.clone(),
        ..ports(Arc::new(FakeGitHub::answering([])), now())
    };
    let mut app = app();
    let (worker, release) = held_survey_worker(ports, &mut app);

    for _ in 0..MORE_THAN_THE_QUEUE {
        assert_eq!(
            dispatch(&worker, &mut app, vec![Effect::CheckJobs]),
            Flow::Continue
        );
    }
    let chosen = UpstreamTargets::new([repo_id("octo/app")]);
    dispatch(&worker, &mut app, vec![store(&chosen)]);
    assert!(worker.finish(&mut app, PATIENCE).is_ok());
    assert_eq!(upstreams.stored(), [chosen]);
    release.send(()).unwrap();
}

/// Quitting right after a choice waits for its write to land.
#[test]
fn quitting_waits_for_queued_writes() {
    let (gate, started, release) = Gate::new();
    let upstreams = Arc::new(GatedUpstreams {
        gate,
        inner: FakeUpstreams::default(),
    });
    let ports = Ports {
        upstreams: upstreams.clone(),
        ..ports(Arc::new(FakeGitHub::answering([])), now())
    };
    let mut app = app();
    let worker = Worker::start(ports);
    let chosen = UpstreamTargets::new([repo_id("octo/app")]);
    let flow = dispatch(&worker, &mut app, vec![store(&chosen), Effect::Quit]);
    assert_eq!(flow, Flow::Quit);
    started.recv_timeout(PATIENCE).expect("the write began");
    assert!(upstreams.inner.stored().is_empty(), "held at the gate");

    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        release.send(()).unwrap();
    });
    assert!(worker.finish(&mut app, PATIENCE).is_ok());
    assert_eq!(upstreams.inner.stored(), [chosen]);
    releaser.join().unwrap();
}

/// A write that never returns does not hold den open: quitting gives up after
/// its bound and says so.
#[test]
fn quitting_gives_up_on_a_hung_write_after_its_bound() {
    const BOUND: Duration = Duration::from_millis(100);
    let (gate, started, release) = Gate::new();
    let upstreams = Arc::new(GatedUpstreams {
        gate,
        inner: FakeUpstreams::default(),
    });
    let ports = Ports {
        upstreams: upstreams.clone(),
        ..ports(Arc::new(FakeGitHub::answering([])), now())
    };
    let mut app = app();
    let worker = Worker::start(ports);
    dispatch(
        &worker,
        &mut app,
        vec![store(&UpstreamTargets::new([repo_id("octo/app")]))],
    );
    started.recv_timeout(PATIENCE).expect("the write began");

    let began = Instant::now();
    let finished = worker.finish(&mut app, BOUND);
    assert!(
        matches!(finished, Err(FinishError::TimedOut { waited }) if waited == BOUND),
        "{finished:?}"
    );
    assert!(began.elapsed() < PATIENCE, "returned near its bound");
    assert!(upstreams.inner.stored().is_empty());
    release.send(()).unwrap();
}
