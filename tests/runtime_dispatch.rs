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
        // Surveys still queued when a test ends find nobody listening: fine.
        let _ = self.started.lock().unwrap().send(());
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
fn an_upstream_choice_is_stored_at_once_even_while_a_survey_runs() {
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

/// An Effect the worker cannot accept (its queue is full behind a running
/// survey) is handed back as its rejection Event, for the App to recover from,
/// never dropped.
#[test]
fn an_effect_the_busy_worker_rejects_comes_back_as_its_rejection_event() {
    /// More than the worker's queue holds; any bound past it will do.
    const MORE_THAN_THE_QUEUE: usize = 1024;
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let ports = Ports {
        github: Arc::new(HeldSurvey {
            started: Mutex::new(started_tx),
            release: Mutex::new(release_rx),
        }),
        ..ports(Arc::new(FakeGitHub::answering([])), now())
    };
    let worker = Worker::start(ports.clone());
    let survey = |id| Effect::Survey {
        id: SurveyId(id),
        request: SurveyRequest {
            owners: vec!["octo".to_owned()],
            upstreams: UpstreamTargets::default(),
        },
    };
    dispatch(vec![survey(0)], &worker, &ports);
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the survey started on the worker");

    let rejected = (1..=MORE_THAN_THE_QUEUE as u64)
        .find_map(|id| {
            let dispatched = dispatch(vec![survey(id)], &worker, &ports);
            assert!(!dispatched.quit);
            dispatched.events.into_iter().next()
        })
        .expect("the full queue rejects an Effect");

    assert!(
        matches!(
            &rejected,
            Event::EffectRejected { effect, reason }
                if matches!(**effect, Effect::Survey { .. }) && reason.contains("still busy")
        ),
        "{rejected:?}"
    );
    release_tx.send(()).unwrap();
}
