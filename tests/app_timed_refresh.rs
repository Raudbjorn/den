//! Seam 2, timed refreshes (#57): with `refresh_interval_secs` set, den
//! surveys again, GitHub and Linear together as `r` does, once that long has
//! passed since the last survey answered. Off by default. At most one survey
//! is in flight, missed ticks never pile up, a rate limit waits for its
//! reset, a failure keeps the last good data on screen and waits the full
//! interval, and an open launch sheet pauses it. Driven by Ticks on a fake
//! clock.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Event, Key, view_model::Overlay},
    config::Config,
    domain::Observation,
    ports::SurveyFailure,
};
use jiff::{SignedDuration, Timestamp};
use support::{
    build::{ObservationBuilder, now, pr},
    harness::{Harness, config, config_with_linear},
};

/// The interval the tests set, in seconds: den's shortest.
const INTERVAL_SECS: i64 = 60;

fn at(secs: i64) -> Timestamp {
    now() + SignedDuration::from_secs(secs)
}

fn observation() -> Observation {
    ObservationBuilder::new()
        .fully_read_owner("alpha")
        .repo("alpha/app")
        .pull_request(pr("alpha/app", 1).author("someone").title("one"))
        .pull_request(pr("alpha/app", 2).author("someone").title("two"))
        .build()
}

fn every(secs: Option<i64>) -> Config {
    Config {
        refresh_interval_secs: secs.map(|s| u64::try_from(s).unwrap()),
        ..config(&["alpha"])
    }
}

/// Started at [`now`]; the startup survey answered then.
fn started(config: Config) -> Harness {
    let mut harness = Harness::new(Ok(config));
    harness.answer(Ok(observation()));
    harness.start();
    assert_eq!(harness.survey_effects(), 1, "the startup survey");
    harness
}

/// A Tick at `secs` after [`now`]; its Effects, not performed.
fn tick(harness: &mut Harness, secs: i64) -> Vec<Effect> {
    harness.send(Event::Tick { now: at(secs) })
}

fn surveys(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Survey { .. }))
        .count()
}

fn linear_surveys(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::SurveyLinear { .. }))
        .count()
}

/// Answers the survey among `effects` at `secs` after [`now`] with `outcome`.
fn answer_at(
    harness: &mut Harness,
    effects: &[Effect],
    secs: i64,
    outcome: Result<Observation, SurveyFailure>,
) {
    harness.answer(outcome);
    harness.set_clock(at(secs));
    let survey = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .expect("a survey to answer")
        .clone();
    harness.perform(&survey);
}

#[test]
fn without_the_setting_den_never_surveys_on_its_own() {
    let mut harness = started(every(None));
    for secs in [INTERVAL_SECS, 3_600, 86_400] {
        assert_eq!(surveys(&tick(&mut harness, secs)), 0);
    }
    assert_eq!(harness.survey_effects(), 1);
}

#[test]
fn the_interval_after_the_last_answer_starts_one_survey_with_its_jobs_check() {
    let mut harness = started(every(Some(INTERVAL_SECS)));
    assert_eq!(surveys(&tick(&mut harness, INTERVAL_SECS - 1)), 0);
    let effects = tick(&mut harness, INTERVAL_SECS);
    assert_eq!(surveys(&effects), 1);
    assert!(
        effects.iter().any(|e| matches!(e, Effect::CheckJobs)),
        "as r does: {effects:?}"
    );
}

/// Decision 2 on #57: Linear refreshes on the same timer.
#[test]
fn linear_is_surveyed_on_the_same_timer() {
    let config = Config {
        refresh_interval_secs: Some(60),
        ..config_with_linear(&["alpha"], &[("alpha", "alpha")])
    };
    let mut harness = Harness::new(Ok(config));
    harness.answer(Ok(observation()));
    harness.start();
    let effects = tick(&mut harness, INTERVAL_SECS);
    assert_eq!(surveys(&effects), 1);
    assert_eq!(linear_surveys(&effects), 1);
}

/// One survey in flight at a time: a Tick while it runs starts nothing, and
/// the next is due the interval after it answers, not after it started.
#[test]
fn a_survey_in_flight_is_never_joined_by_another_and_the_next_counts_from_its_answer() {
    let mut harness = started(every(Some(INTERVAL_SECS)));
    let running = tick(&mut harness, INTERVAL_SECS);
    assert_eq!(surveys(&running), 1);
    for secs in [2 * INTERVAL_SECS, 10 * INTERVAL_SECS] {
        assert_eq!(surveys(&tick(&mut harness, secs)), 0, "still running");
    }

    let answered = 10 * INTERVAL_SECS + 5;
    answer_at(&mut harness, &running, answered, Ok(observation()));
    assert_eq!(
        surveys(&tick(&mut harness, answered + INTERVAL_SECS - 1)),
        0
    );
    assert_eq!(surveys(&tick(&mut harness, answered + INTERVAL_SECS)), 1);
}

/// Missed refreshes never pile up: after a long quiet spell one Tick starts
/// one survey, and the next Tick starts none.
#[test]
fn a_long_gap_starts_one_survey_not_one_per_missed_interval() {
    let mut harness = started(every(Some(INTERVAL_SECS)));
    let late = 24 * 3_600;
    assert_eq!(surveys(&tick(&mut harness, late)), 1);
    assert_eq!(surveys(&tick(&mut harness, late + 1)), 0);
    assert_eq!(harness.survey_effects(), 2);
}

/// A failed refresh keeps the last good survey on screen, says it failed,
/// and waits a full interval before trying again.
#[test]
fn a_failed_refresh_keeps_the_last_data_and_waits_the_full_interval() {
    let mut harness = started(every(Some(INTERVAL_SECS)));
    let rows_before = harness.rows();
    let running = tick(&mut harness, INTERVAL_SECS);
    let failed_at = INTERVAL_SECS + 2;
    answer_at(
        &mut harness,
        &running,
        failed_at,
        Err(SurveyFailure::Interrupted {
            detail: "connection reset".to_owned(),
        }),
    );
    assert_eq!(harness.rows(), rows_before, "the last good data stays");
    // The header says the last refresh failed (with the failure's headline).
    assert!(
        format!("{:?}", harness.view).contains("last refresh failed"),
        "the failure is shown: {:?}",
        harness.view
    );
    assert_eq!(
        surveys(&tick(&mut harness, failed_at + INTERVAL_SECS - 1)),
        0
    );
    assert_eq!(surveys(&tick(&mut harness, failed_at + INTERVAL_SECS)), 1);
}

/// GitHub's rate limit: no timed refresh before it resets.
#[test]
fn a_rate_limited_refresh_waits_for_the_reset() {
    let mut harness = started(every(Some(INTERVAL_SECS)));
    let running = tick(&mut harness, INTERVAL_SECS);
    let reset = 15 * INTERVAL_SECS;
    answer_at(
        &mut harness,
        &running,
        INTERVAL_SECS + 1,
        Err(SurveyFailure::RateLimited {
            resets_at: Some(at(reset)),
        }),
    );
    assert_eq!(surveys(&tick(&mut harness, 3 * INTERVAL_SECS)), 0);
    assert_eq!(surveys(&tick(&mut harness, reset - 1)), 0);
    assert_eq!(surveys(&tick(&mut harness, reset)), 1);
}

/// Decision 3 on #57: an open launch sheet pauses timed refreshes, so the
/// sheet never asks again unprompted. Closing it lets the overdue one run.
#[test]
fn an_open_launch_sheet_pauses_the_timer_until_it_closes() {
    let mut harness = started(every(Some(INTERVAL_SECS)));
    harness.key(Key::Char('c'));
    assert!(
        matches!(harness.view.overlay, Some(Overlay::LaunchSheet(_))),
        "{:?}",
        harness.view.overlay
    );
    for secs in [INTERVAL_SECS, 10 * INTERVAL_SECS] {
        assert_eq!(surveys(&tick(&mut harness, secs)), 0, "paused");
    }
    harness.key(Key::Esc);
    assert_eq!(harness.view.overlay, None);
    assert_eq!(surveys(&tick(&mut harness, 10 * INTERVAL_SECS + 1)), 1);
}

/// The timed refresh is `r`'s: the selection stays on the same Work Item.
#[test]
fn a_timed_refresh_keeps_the_selection() {
    let mut harness = started(every(Some(INTERVAL_SECS)));
    harness.key(Key::Down);
    let selected = harness.view.list.selected;
    assert_eq!(selected, Some(1));
    let reference = harness.view.list.rows[1].reference.clone();
    let running = tick(&mut harness, INTERVAL_SECS);
    answer_at(&mut harness, &running, INTERVAL_SECS + 1, Ok(observation()));
    assert_eq!(harness.view.list.selected, selected);
    assert_eq!(harness.view.list.rows[1].reference, reference);
}

/// A Linear survey still in flight also holds the timer: GitHub answering
/// first doesn't start a second Linear survey beside the first.
#[test]
fn a_linear_survey_still_running_holds_the_timer() {
    let config = Config {
        refresh_interval_secs: Some(60),
        ..config_with_linear(&["alpha"], &[("alpha", "alpha")])
    };
    let mut harness = Harness::new(Ok(config));
    harness.answer(Ok(observation()));
    harness.start();
    let running = tick(&mut harness, INTERVAL_SECS);
    assert_eq!(linear_surveys(&running), 1);
    // GitHub answers; Linear's survey is never performed, so it runs on.
    answer_at(&mut harness, &running, INTERVAL_SECS + 1, Ok(observation()));
    for secs in [3 * INTERVAL_SECS, 30 * INTERVAL_SECS] {
        let effects = tick(&mut harness, secs);
        assert_eq!(surveys(&effects), 0, "Linear still running");
        assert_eq!(linear_surveys(&effects), 0);
    }
}
