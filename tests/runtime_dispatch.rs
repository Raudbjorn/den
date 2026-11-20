//! The interactive runtime's dispatch: which Effects wait for the background
//! worker and which happen at once.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Event, SurveyId},
    domain::{Observation, upstream::UpstreamTargets},
    ports::{GitHubSource, Ports, SurveyFailure, SurveyRequest},
    runtime::{Worker, dispatch},
};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};
use support::{
    build::{now, repo_id},
    fakes::{FakeGitHub, FakeUpstreams, ports},
};

/// A GitHub whose survey runs until the test releases it, as a slow survey would.
struct HeldSurvey {
    started: Mutex<Sender<()>>,
    release: Mutex<Receiver<()>>,
}

impl GitHubSource for HeldSurvey {
    fn survey(&self, _: &SurveyRequest) -> Result<Observation, SurveyFailure> {
        self.started.lock().unwrap().send(()).unwrap();
        self.release.lock().unwrap().recv().ok();
        Err(SurveyFailure::NotLoggedIn {
            detail: "released".to_owned(),
        })
    }
}

/// An Upstream choice made while a survey runs is stored before the survey
/// ends: quitting does not wait for the worker, so a choice queued behind the
/// survey would be lost.
#[test]
fn an_upstream_choice_is_stored_at_once_even_while_the_worker_queue_is_full() {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let upstreams = Arc::new(FakeUpstreams::default());
    let ports = Ports {
        github: Arc::new(HeldSurvey {
            started: Mutex::new(started_tx),
            release: Mutex::new(release_rx),
        }),
        upstreams: upstreams.clone(),
        ..ports(Arc::new(FakeGitHub::answering([])), now())
    };
    let worker = Worker::start(ports.clone());
    let survey = Effect::Survey {
        id: SurveyId(0),
        request: SurveyRequest {
            owners: vec!["octo".to_owned()],
            upstreams: UpstreamTargets::default(),
        },
    };
    dispatch(vec![survey], &worker, &ports);
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the survey started on the worker");

    // Exceed the worker's 16-slot queue while its survey is held.
    let saturated = dispatch(vec![Effect::CheckJobs; 64], &worker, &ports);
    assert!(!saturated.events.is_empty(), "the queue must be full");
    assert!(
        saturated.events.iter().all(|event| matches!(event,
            Event::EffectRejected { effect, reason }
                if **effect == Effect::CheckJobs && reason.contains("still busy")
        )),
        "rejected operations must reach the App: {:?}",
        saturated.events
    );

    let chosen = UpstreamTargets::new([repo_id("octo/app")]);
    let dispatched = dispatch(
        vec![Effect::StoreUpstreams {
            upstreams: chosen.clone(),
        }],
        &worker,
        &ports,
    );

    assert_eq!(upstreams.stored(), [chosen], "stored while the survey runs");
    assert!(
        matches!(
            dispatched.events.as_slice(),
            [Event::UpstreamsStored { outcome: Ok(()) }]
        ),
        "{:?}",
        dispatched.events
    );
    assert!(!dispatched.quit);
    release_tx.send(()).unwrap();
}

#[test]
fn quit_is_reported_and_nothing_else_is_performed_at_once() {
    let upstreams = Arc::new(FakeUpstreams::default());
    let ports = Ports {
        upstreams: upstreams.clone(),
        ..ports(Arc::new(FakeGitHub::answering([])), now())
    };
    let worker = Worker::start(ports.clone());
    let dispatched = dispatch(vec![Effect::Quit], &worker, &ports);
    assert!(dispatched.quit);
    assert!(dispatched.events.is_empty());
    assert!(upstreams.stored().is_empty());
}
