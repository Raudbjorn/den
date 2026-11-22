//! The cache's read-time guard (spec 0003, "Cache"; ADR-0005): for each Repo
//! and source, a read is merged only if it didn't start before the one den
//! holds. An answer that started earlier changes nothing for that Repo and
//! source: not its facts, its coverage, its age or its read time, whether it
//! is complete or partial. Pure merges first, then the App (seam 2) showing a
//! cache another den wrote after this one's survey started.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    cache::{self, CachedObservation},
    domain::{Incompleteness, Observation, Source, SourceKind, SourceState},
};
use support::{
    build::{ME, ObservationBuilder, now, pr, repo_id, ts},
    fakes::FakeCache,
    harness::Harness,
};

/// A Repo read ten minutes after `now()` (another den's, say).
const LATER: &str = "2026-11-19T12:10:00Z";
/// After that again.
const LATEST: &str = "2026-11-19T12:20:00Z";

/// Every source of `repo` recorded for the Repo itself, as both surveys do
/// since coverage is per Repo.
fn read_whole(
    mut builder: ObservationBuilder,
    repo: &str,
    state: &SourceState,
) -> ObservationBuilder {
    builder = builder.repo(repo);
    for kind in [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::Issues,
        SourceKind::Branches,
        SourceKind::Runs,
    ] {
        builder = builder.repo_coverage(repo, kind, state.clone());
    }
    builder
}

/// What den holds: acme/app read at `LATER`, its PR #1 titled "held".
fn held() -> CachedObservation {
    let observation = read_whole(
        ObservationBuilder::new()
            .observed_at(LATER)
            .fully_read_owner("acme"),
        "acme/app",
        &SourceState::Complete,
    )
    .pull_request(pr("acme/app", 1).author(ME).title("held"))
    .build();
    CachedObservation::fresh(observation)
}

/// An Owner survey of acme that started at `at`, with acme/app in `app_state`:
/// #1 titled "answer" and a #2, and acme/lib's #3.
fn owner_survey(at: &str, app_state: &SourceState) -> Observation {
    let builder = ObservationBuilder::new()
        .observed_at(at)
        .fully_read_owner("acme");
    let builder = read_whole(builder, "acme/app", app_state);
    read_whole(builder, "acme/lib", &SourceState::Complete)
        .pull_request(pr("acme/app", 1).author(ME).title("answer"))
        .pull_request(pr("acme/app", 2).author(ME))
        .pull_request(pr("acme/lib", 3).author(ME))
        .build()
}

/// An Owner survey of acme that started at `at` and was cut off (rate
/// limited) before its listing reached acme/app: acme's sources are partial,
/// and only acme/lib has entries of its own.
fn owner_survey_cut_before_app(at: &str) -> Observation {
    let builder = [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::Issues,
        SourceKind::Branches,
        SourceKind::Runs,
    ]
    .into_iter()
    .fold(
        ObservationBuilder::new().observed_at(at).owner("acme"),
        |builder, kind| builder.owner_coverage("acme", kind, rate_limited()),
    );
    read_whole(builder, "acme/lib", &SourceState::Complete)
        .pull_request(pr("acme/lib", 3).author(ME))
        .build()
}

fn titles(cached: &CachedObservation) -> Vec<(String, u64, String)> {
    let mut titles: Vec<(String, u64, String)> = cached
        .observation
        .pull_requests
        .iter()
        .map(|p| (p.repo.to_string(), p.number, p.title.clone()))
        .collect();
    titles.sort();
    titles
}

fn state(cached: &CachedObservation, repo: &str, kind: SourceKind) -> Option<SourceState> {
    cached
        .observation
        .coverage
        .get(&Source::Repo {
            repo: repo_id(repo),
            kind,
        })
        .cloned()
}

fn rate_limited() -> SourceState {
    SourceState::Partial(Incompleteness::RateLimited {
        resets_at: Some(ts("2026-11-19T13:00:00Z")),
    })
}

#[test]
fn an_answer_that_started_before_the_held_read_changes_nothing_of_that_repo() {
    let merged = cache::merge(
        Some(held()),
        owner_survey("2026-11-19T12:00:00Z", &SourceState::Complete),
    );

    assert_eq!(
        titles(&merged),
        [
            ("acme/app".to_owned(), 1, "held".to_owned()),
            ("acme/lib".to_owned(), 3, "PR 3".to_owned()),
        ],
        "acme/app's PRs stay as held; acme/lib's are the answer's"
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(ts(LATER))
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/lib"), SourceKind::PullRequests),
        Some(now())
    );
    // Nothing reads newer than it is: the Observation is as new as the held
    // read, and the older answer's own sources carry its age.
    assert_eq!(merged.observation.observed_at, ts(LATER));
    let lib = Source::Repo {
        repo: repo_id("acme/lib"),
        kind: SourceKind::PullRequests,
    };
    assert_eq!(merged.carried_since(&lib), Some(now()));
    let app = Source::Repo {
        repo: repo_id("acme/app"),
        kind: SourceKind::PullRequests,
    };
    assert_eq!(merged.carried_since(&app), None);
}

#[test]
fn a_partial_answer_that_started_earlier_neither_replaces_nor_downgrades_the_held_read() {
    let merged = cache::merge(
        Some(held()),
        owner_survey("2026-11-19T12:00:00Z", &rate_limited()),
    );

    assert!(titles(&merged).contains(&("acme/app".to_owned(), 1, "held".to_owned())));
    assert!(
        !titles(&merged)
            .iter()
            .any(|(repo, number, _)| repo == "acme/app" && *number == 2)
    );
    for kind in [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::Runs,
    ] {
        assert_eq!(
            state(&merged, "acme/app", kind),
            Some(SourceState::Complete),
            "{kind:?} keeps the held coverage"
        );
    }
}

#[test]
fn an_earlier_answer_that_never_reached_the_held_repo_keeps_its_coverage_and_read_time() {
    let merged = cache::merge(
        Some(held()),
        owner_survey_cut_before_app("2026-11-19T12:00:00Z"),
    );

    assert!(titles(&merged).contains(&("acme/app".to_owned(), 1, "held".to_owned())));
    for kind in [
        SourceKind::Repos,
        SourceKind::PullRequests,
        SourceKind::Issues,
        SourceKind::Branches,
        SourceKind::Runs,
    ] {
        assert_eq!(
            state(&merged, "acme/app", kind),
            Some(SourceState::Complete),
            "{kind:?} keeps the held coverage"
        );
        assert_eq!(
            merged.read_started(&repo_id("acme/app"), kind),
            Some(ts(LATER)),
            "{kind:?} keeps the held read time"
        );
    }

    // The read time survived, so a still earlier answer that does reach
    // acme/app is refused too.
    let again = cache::merge(
        Some(merged),
        owner_survey("2026-11-19T11:00:00Z", &SourceState::Complete),
    );
    assert!(titles(&again).contains(&("acme/app".to_owned(), 1, "held".to_owned())));
    assert!(
        !titles(&again)
            .iter()
            .any(|(repo, number, _)| repo == "acme/app" && *number == 2)
    );
}

#[test]
fn an_answer_that_started_after_the_held_read_replaces_it() {
    let merged = cache::merge(Some(held()), owner_survey(LATEST, &SourceState::Complete));

    assert_eq!(
        titles(&merged),
        [
            ("acme/app".to_owned(), 1, "answer".to_owned()),
            ("acme/app".to_owned(), 2, "PR 2".to_owned()),
            ("acme/lib".to_owned(), 3, "PR 3".to_owned()),
        ]
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(ts(LATEST))
    );
    assert_eq!(merged.observation.observed_at, ts(LATEST));
}

#[test]
fn the_guard_matches_a_repo_whatever_its_case() {
    let observation = read_whole(
        ObservationBuilder::new()
            .observed_at(LATER)
            .fully_read_owner("acme"),
        "Acme/App",
        &SourceState::Complete,
    )
    .pull_request(pr("Acme/App", 1).author(ME).title("held"))
    .build();
    let merged = cache::merge(
        Some(CachedObservation::fresh(observation)),
        owner_survey("2026-11-19T12:00:00Z", &SourceState::Complete),
    );

    // A kept fact takes the Owner's spelling from the refresh, as every
    // kept fact does; the guard held it all the same.
    let app: Vec<(u64, String)> = titles(&merged)
        .into_iter()
        .filter(|(repo, _, _)| repo.eq_ignore_ascii_case("acme/app"))
        .map(|(_, number, title)| (number, title))
        .collect();
    assert_eq!(app, [(1, "held".to_owned())]);
}

#[test]
fn a_cached_read_another_den_made_later_survives_this_dens_earlier_survey() {
    let mut harness = Harness::with_cache(
        Ok(support::harness::config(&["acme"])),
        FakeCache::holding(held()),
    );
    harness.answer(Ok(owner_survey(
        "2026-11-19T12:00:00Z",
        &SourceState::Complete,
    )));
    harness.start();

    let rows: Vec<(String, String)> = harness
        .view
        .list
        .rows
        .iter()
        .map(|r| (format!("{}{}", r.repo, r.reference), r.title.clone()))
        .collect();
    assert!(
        rows.contains(&("acme/app#1".to_owned(), "held".to_owned())),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|(item, _)| item == "acme/app#2"),
        "{rows:?}"
    );
    let stored = harness.cache.stored();
    let last = stored.last().expect("the merge is saved");
    assert_eq!(
        last.observation.observed_at,
        ts(LATER),
        "as new as what it holds"
    );
}

/// Equal start times merge (spec 0003, "Merge"; the maintainer's ruling on
/// Plan 2's Decision 4, 2026-11-22): the guard refuses only a held read that
/// started strictly later, so an answer that started at the same moment as
/// the held read replaces it.
#[test]
fn an_answer_that_started_when_the_held_read_did_replaces_it() {
    let merged = cache::merge(Some(held()), owner_survey(LATER, &SourceState::Complete));

    assert_eq!(
        titles(&merged),
        [
            ("acme/app".to_owned(), 1, "answer".to_owned()),
            ("acme/app".to_owned(), 2, "PR 2".to_owned()),
            ("acme/lib".to_owned(), 3, "PR 3".to_owned()),
        ]
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(ts(LATER))
    );
}

/// When the guard keeps the Observation as new as the held read, the
/// refresh's own sources carry the refresh's age, but only those holding
/// data: a source the refresh couldn't read at all (acme/lib's Pull Requests
/// failed, and den held nothing of acme/lib, not even through acme's
/// coverage, since it held a read of acme/app alone) has no data to date, so
/// it gets no age.
#[test]
fn a_guarded_merge_dates_only_the_refreshs_sources_that_hold_data() {
    let failed = SourceState::Failed {
        error: "boom".to_owned(),
    };
    let builder = ObservationBuilder::new()
        .observed_at("2026-11-19T12:00:00Z")
        .fully_read_owner("acme");
    let builder = read_whole(builder, "acme/app", &SourceState::Complete);
    let fresh = read_whole(builder, "acme/lib", &SourceState::Complete)
        .repo_coverage("acme/lib", SourceKind::PullRequests, failed.clone())
        .pull_request(pr("acme/app", 1).author(ME).title("answer"))
        .build();

    let app_alone = read_whole(
        ObservationBuilder::new().observed_at(LATER),
        "acme/app",
        &SourceState::Complete,
    )
    .pull_request(pr("acme/app", 1).author(ME).title("held"))
    .build();
    let merged = cache::merge(Some(CachedObservation::fresh(app_alone)), fresh);

    assert_eq!(merged.observation.observed_at, ts(LATER), "as new as held");
    let lib = |kind| Source::Repo {
        repo: repo_id("acme/lib"),
        kind,
    };
    assert_eq!(
        state(&merged, "acme/lib", SourceKind::PullRequests),
        Some(failed)
    );
    assert_eq!(
        merged.carried_since(&lib(SourceKind::PullRequests)),
        None,
        "no data, so no age"
    );
    assert_eq!(
        merged.carried_since(&lib(SourceKind::Issues)),
        Some(now()),
        "the refresh's data is as old as the refresh"
    );
}
