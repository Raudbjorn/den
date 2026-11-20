//! Seam 2: the cache, refresh and honest coverage (#8). The App is driven by
//! events with fake GitHub and cache ports; assertions are on the ViewModel and
//! the Effects only.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Event, Key, LogLevel, ViewModel},
    cache::{CacheError, CachedObservation},
    domain::{Incompleteness, Observation, SourceKind, SourceState},
    ports::SurveyFailure,
};
use std::path::PathBuf;
use support::{
    build::{ME, ObservationBuilder, now, pr, ts},
    fakes::FakeCache,
    harness::{Harness, config},
};

/// When the earlier survey was observed (two hours before `now()`).
const EARLIER: &str = "2026-11-19T10:00:00Z";

fn references(view: &ViewModel) -> Vec<String> {
    view.list
        .rows
        .iter()
        .map(|r| format!("{}{}", r.repo, r.reference))
        .collect()
}

fn effect_named(effects: &[Effect], wanted: fn(&Effect) -> bool) -> Effect {
    effects
        .iter()
        .find(|e| wanted(e))
        .cloned()
        .unwrap_or_else(|| panic!("no such effect in {effects:?}"))
}

fn failed(error: &str) -> SourceState {
    SourceState::Failed {
        error: error.to_owned(),
    }
}

fn rate_limited(at: Option<&str>) -> SourceState {
    SourceState::Partial(Incompleteness::RateLimited {
        resets_at: at.map(ts),
    })
}

/// An earlier, complete survey of alpha and beta with one PR each.
fn earlier_survey() -> Observation {
    ObservationBuilder::new()
        .observed_at(EARLIER)
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .repo("alpha/app")
        .repo("beta/lib")
        .pull_request(pr("alpha/app", 1).title("Old alpha PR"))
        .pull_request(pr("beta/lib", 2).title("Old beta PR"))
        .build()
}

/// A harness for alpha and beta that has already shown `earlier_survey()`.
fn after_earlier_survey() -> Harness {
    let mut harness = Harness::with_owners(&["alpha", "beta"]);
    harness.answer(Ok(earlier_survey()));
    harness.start();
    harness
}

/// Presses `r` and performs the refresh it asks for.
fn refresh(harness: &mut Harness) -> Vec<Effect> {
    let effects = harness.key(Key::Char('r'));
    let survey = effect_named(&effects, |e| matches!(e, Effect::Survey { .. }));
    let mut follow_ups = harness.perform(&survey);
    for effect in follow_ups.clone() {
        follow_ups.extend(harness.perform(&effect));
    }
    follow_ups
}

#[test]
fn startup_shows_the_cached_observation_and_its_age_before_the_survey_answers() {
    let cache = FakeCache::holding(CachedObservation::fresh(earlier_survey()));
    let mut harness = Harness::with_cache(Ok(config(&["alpha", "beta"])), cache);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .pull_request(pr("alpha/app", 3).title("New PR"))
        .build()));

    let effects = harness.send(Event::Started { now: now() });
    let load = effects
        .iter()
        .position(|e| matches!(e, Effect::LoadCache))
        .expect("the cache is read at startup");
    let survey = effects
        .iter()
        .position(|e| matches!(e, Effect::Survey { .. }))
        .expect("and a refresh starts");
    assert!(load < survey, "the cache is read before the network");

    harness.perform(&Effect::LoadCache);
    let view = harness.view.clone();
    assert_eq!(references(&view), ["alpha/app#1", "beta/lib#2"]);
    assert!(
        view.header.status.contains("cached")
            && view.header.status.contains("10:00 UTC")
            && view.header.status.contains("2 h ago")
            && view.header.status.contains("refreshing"),
        "{}",
        view.header.status
    );

    let follow_ups = harness.perform(&effects[survey]);
    assert_eq!(
        references(&harness.view),
        ["alpha/app#3"],
        "a complete refresh replaces the cached data"
    );
    let status = &harness.view.header.status;
    assert!(
        status.contains("12:00 UTC")
            && !status.contains("cached")
            && !status.contains("refreshing"),
        "{status}"
    );
    let Effect::StoreCache { cached } =
        effect_named(&follow_ups, |e| matches!(e, Effect::StoreCache { .. }))
    else {
        unreachable!()
    };
    assert_eq!(cached.observation.pull_requests.len(), 1);
    assert!(cached.carried.is_empty());
}

#[test]
fn a_refresh_that_fails_keeps_the_old_data_and_shows_the_failure() {
    let mut harness = after_earlier_survey();
    let stored_before = harness.cache.stored().len();
    harness.answer(Err(SurveyFailure::ViewerUnavailable {
        detail: "dial tcp: no such host".to_owned(),
    }));
    let effects = refresh(&mut harness);

    assert_eq!(references(&harness.view), ["alpha/app#1", "beta/lib#2"]);
    let status = &harness.view.header.status;
    assert!(
        status.contains("last refresh failed") && status.contains("10:00 UTC"),
        "{status}"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::StoreCache { .. })),
        "a failed refresh never overwrites the cache"
    );
    assert_eq!(harness.cache.stored().len(), stored_before);
}

#[test]
fn a_source_that_failed_keeps_its_previous_data_marked_with_its_age_and_the_failure() {
    let mut harness = after_earlier_survey();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .owner_coverage("beta", SourceKind::PullRequests, failed("gh timed out"))
        .repo("alpha/app")
        .repo("beta/lib")
        .pull_request(pr("alpha/app", 5).title("New alpha PR"))
        .build()));
    refresh(&mut harness);
    let view = harness.view.clone();

    assert_eq!(
        references(&view),
        ["alpha/app#5", "beta/lib#2"],
        "alpha was read completely, so #1 is gone; beta failed, so #2 is kept"
    );
    let kept = view
        .list
        .rows
        .iter()
        .find(|r| r.reference == "#2")
        .expect("kept");
    assert!(kept.incomplete);
    assert!(!view.list.rows[0].incomplete);

    assert!(!view.header.coverage_complete);
    assert!(
        view.header.coverage.contains("Pull Requests of beta")
            && view.header.coverage.contains("gh timed out")
            && view.header.coverage.contains("10:00 UTC"),
        "{}",
        view.header.coverage
    );

    harness.key(Key::Down);
    let detail = harness.view.detail.content.clone().expect("selected");
    assert_eq!(detail.reference, "beta/lib#2");
    assert!(
        detail
            .coverage
            .iter()
            .any(|c| c.contains("failed") && c.contains("10:00 UTC") && c.contains("2 h ago")),
        "{:?}",
        detail.coverage
    );
}

#[test]
fn data_kept_twice_keeps_its_original_age() {
    let mut harness = after_earlier_survey();
    for _ in 0..2 {
        harness.answer(Ok(ObservationBuilder::new()
            .observed_at("2026-11-19T11:00:00Z")
            .fully_read_owner("alpha")
            .fully_read_owner("beta")
            .owner_coverage("beta", SourceKind::PullRequests, failed("boom"))
            .build()));
        refresh(&mut harness);
    }
    assert!(
        harness.view.header.coverage.contains("10:00 UTC"),
        "{}",
        harness.view.header.coverage
    );
    assert!(references(&harness.view).contains(&"beta/lib#2".to_owned()));
}

#[test]
fn a_rate_limited_refresh_keeps_the_old_data_and_shows_the_reset_time() {
    // Rate limited before den knew who you are: no Observation at all.
    let mut harness = after_earlier_survey();
    harness.answer(Err(SurveyFailure::RateLimited {
        resets_at: Some(ts("2026-11-19T12:45:00Z")),
    }));
    refresh(&mut harness);
    assert_eq!(references(&harness.view), ["alpha/app#1", "beta/lib#2"]);
    assert!(
        harness.view.header.status.contains("rate limit")
            && harness.view.header.status.contains("12:45 UTC"),
        "{}",
        harness.view.header.status
    );

    // Rate limited mid-survey (GraphQL says so with HTTP 200): partial.
    let mut harness = after_earlier_survey();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .owner_coverage(
            "beta",
            SourceKind::PullRequests,
            rate_limited(Some("2026-11-19T13:00:00Z")),
        )
        .pull_request(pr("alpha/app", 1).title("Old alpha PR"))
        .build()));
    refresh(&mut harness);
    let view = &harness.view;
    assert_eq!(references(view), ["alpha/app#1", "beta/lib#2"]);
    assert!(
        view.header.status.contains("rate limited until 13:00 UTC"),
        "{}",
        view.header.status
    );
    assert!(!view.header.coverage_complete);
    assert!(view.header.coverage.contains("rate limited"));
}

#[test]
fn an_unknown_reset_time_is_said_not_invented() {
    let mut harness = after_earlier_survey();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .owner_coverage("alpha", SourceKind::Repos, rate_limited(None))
        .build()));
    refresh(&mut harness);
    assert!(
        harness
            .view
            .header
            .status
            .contains("GitHub did not say when it resets"),
        "{}",
        harness.view.header.status
    );
}

#[test]
fn an_empty_complete_refresh_says_nothing_needs_you_and_an_empty_partial_one_does_not() {
    let mut harness = Harness::with_owners(&["alpha"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .build()));
    harness.start();
    let empty = harness.view.list.empty.clone().expect("empty");
    assert_eq!(empty.title, "Nothing needs you.");
    assert!(harness.view.header.coverage_complete);

    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .owner_coverage("alpha", SourceKind::Issues, rate_limited(None))
        .build()));
    refresh(&mut harness);
    let empty = harness.view.list.empty.clone().expect("still empty");
    assert_ne!(empty.title, "Nothing needs you.");
    assert!(
        empty.detail.contains("Issues of alpha") && empty.detail.contains("rate limited"),
        "says what couldn't be read: {empty:?}"
    );
    assert!(!harness.view.header.coverage_complete);
}

#[test]
fn a_cache_from_other_owners_never_claims_a_complete_survey() {
    let cached = ObservationBuilder::new()
        .observed_at(EARLIER)
        .fully_read_owner("alpha")
        .build();
    let cache = FakeCache::holding(CachedObservation::fresh(cached));
    let mut harness = Harness::with_cache(Ok(config(&["alpha", "gamma"])), cache);
    harness.send(Event::Started { now: now() });
    harness.perform(&Effect::LoadCache);
    let view = &harness.view;
    assert_ne!(
        view.list.empty.as_ref().expect("empty").title,
        "Nothing needs you.",
        "gamma was never surveyed"
    );
    assert!(!view.header.coverage_complete);
}

#[test]
fn an_unreadable_cache_is_a_logged_warning_and_a_fresh_survey() {
    let cache = FakeCache::default();
    cache.answer_load(Err(CacheError::Unreadable {
        path: PathBuf::from("/state/den/cache/me.json"),
        detail: "missing field `facts` at line 1".to_owned(),
    }));
    let mut harness = Harness::with_cache(Ok(config(&["alpha"])), cache);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("alpha")
        .pull_request(pr("alpha/app", 9))
        .build()));
    harness.start();

    assert!(harness.effects.iter().any(|e| matches!(
        e,
        Effect::Log(r) if r.level == LogLevel::Warn && r.message.contains("me.json")
    )));
    assert_eq!(references(&harness.view), ["alpha/app#9"]);
    assert_eq!(
        harness.cache.stored().len(),
        1,
        "the good survey replaces it"
    );
}

#[test]
fn the_cache_is_per_viewer_and_never_merged_across_viewers() {
    let someone_else = ObservationBuilder::new()
        .viewer("someone-else")
        .observed_at(EARLIER)
        .fully_read_owner("alpha")
        .pull_request(pr("alpha/app", 1))
        .build();
    let cache = FakeCache::holding(CachedObservation::fresh(someone_else));
    let mut harness = Harness::with_cache(Ok(config(&["alpha"])), cache);
    harness.answer(Ok(ObservationBuilder::new()
        .viewer(ME)
        .fully_read_owner("alpha")
        .owner_coverage("alpha", SourceKind::PullRequests, failed("boom"))
        .build()));
    harness.start();

    assert!(
        references(&harness.view).is_empty(),
        "another viewer's PRs are not carried into mine"
    );
    let stored = harness.cache.stored();
    let last = stored.last().expect("stored");
    assert_eq!(last.observation.viewer.login, ME);
    assert!(last.carried.is_empty());
}

#[test]
fn r_refreshes_and_the_selection_stays_on_the_same_work_item() {
    let mut harness = Harness::with_owners(&["o"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("o")
        .pull_request(pr("o/r", 1).updated("2026-11-19T11:00:00Z"))
        .pull_request(pr("o/r", 2).updated("2026-11-19T10:00:00Z"))
        .build()));
    harness.start();
    harness.key(Key::Down);
    assert_eq!(harness.view.list.selected, Some(1));
    assert!(
        harness
            .view
            .footer
            .hints
            .iter()
            .any(|h| h.key == "r" && h.label == "refresh")
    );

    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("o")
        .pull_request(pr("o/r", 3).updated("2026-11-19T11:30:00Z"))
        .pull_request(pr("o/r", 1).updated("2026-11-19T11:00:00Z"))
        .pull_request(pr("o/r", 2).updated("2026-11-19T10:00:00Z"))
        .build()));
    refresh(&mut harness);
    assert_eq!(harness.survey_effects(), 2);
    let selected = harness.view.list.selected.expect("selected");
    assert_eq!(harness.view.list.rows[selected].reference, "#2");
    assert_eq!(selected, 2, "it moved down, and the selection followed it");

    // When the selected item is gone, the selection falls back to the first row.
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("o")
        .pull_request(pr("o/r", 3).updated("2026-11-19T11:30:00Z"))
        .build()));
    refresh(&mut harness);
    assert_eq!(harness.view.list.selected, Some(0));
}

#[test]
fn r_while_a_survey_is_running_does_not_start_another() {
    let mut harness = Harness::with_owners(&["o"]);
    harness.send(Event::Started { now: now() });
    let effects = harness.key(Key::Char('r'));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Survey { .. })),
        "{effects:?}"
    );
    assert_eq!(harness.survey_effects(), 1);
}

#[test]
fn coverage_is_shown_per_repo_in_words() {
    let mut harness = Harness::with_owners(&["alpha", "beta"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .repo("alpha/app")
        .repo("beta/lib")
        .repo("beta/big")
        .repo_coverage(
            "beta/big",
            SourceKind::PullRequests,
            SourceState::Partial(Incompleteness::PaginationStopped {
                pages_read: 50,
                reason: "den reads at most 50 pages per list".to_owned(),
            }),
        )
        .build()));
    harness.start();
    let repos: Vec<(String, bool, String)> = harness
        .view
        .header
        .repos
        .iter()
        .map(|r| (r.repo.clone(), r.complete, r.summary.clone()))
        .collect();
    assert_eq!(
        repos,
        [
            ("alpha/app".into(), true, "complete".into()),
            ("beta/lib".into(), true, "complete".into()),
            (
                "beta/big".into(),
                false,
                "incomplete: Pull Requests partial".into()
            ),
        ]
    );
    let big = &harness.view.header.repos[2];
    assert!(
        big.problems
            .iter()
            .any(|p| p.contains("stopped after 50 page(s)")),
        "{:?}",
        big.problems
    );
    assert!(!harness.view.header.coverage_complete);
    assert!(harness.view.header.coverage.contains("beta/big"));
}

#[test]
fn a_kind_not_fetched_yet_marks_every_repo_incomplete() {
    let mut harness = Harness::with_owners(&["alpha"]);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("alpha")
        .repo("alpha/app")
        .build()));
    harness.start();
    let app = &harness.view.header.repos[0];
    assert!(!app.complete);
    // ObservationBuilder::owner reads Issues completely since #17 (as the real
    // source does); Branches and Runs are still left unfetched by it.
    assert_eq!(app.summary, "incomplete: not fetched yet: Branches, Runs");
}

#[test]
fn a_cache_write_failure_is_logged_and_nothing_else_changes() {
    let mut harness = after_earlier_survey();
    let before = harness.view.clone();
    let effects = harness.send(Event::CacheStored {
        outcome: Err(CacheError::Write {
            path: PathBuf::from("/state/den/cache/me.json"),
            detail: "No space left on device".to_owned(),
        }),
    });
    assert!(matches!(
        effects.as_slice(),
        [Effect::Log(r)] if r.level == LogLevel::Warn && r.message.contains("No space left")
    ));
    assert_eq!(harness.view, before);
}
