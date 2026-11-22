//! Merging a Repo Survey into the Repo-keyed cache (spec 0003, "Cache";
//! ADR-0005): it speaks for its own Repo only, by the per-source rule and the
//! read-time guard. Everything else den holds stays, with its own age. And
//! fitting a cache to the config keeps the opened Repo even when its Owner
//! isn't watched.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    cache::{self, CachedObservation},
    domain::{Incompleteness, Observation, RepoId, Source, SourceKind, SourceState},
};
use support::build::{ME, ObservationBuilder, now, pr, repo_id, ts};

/// Two hours before `now()`.
const EARLIER: &str = "2026-11-19T10:00:00Z";
/// Ten minutes after `now()`.
const LATER: &str = "2026-11-19T12:10:00Z";

const KINDS: [SourceKind; 5] = [
    SourceKind::Repos,
    SourceKind::PullRequests,
    SourceKind::Issues,
    SourceKind::Branches,
    SourceKind::Runs,
];

fn read_whole(
    mut builder: ObservationBuilder,
    repo: &str,
    state: &SourceState,
) -> ObservationBuilder {
    builder = builder.repo(repo);
    for kind in KINDS {
        builder = builder.repo_coverage(repo, kind, state.clone());
    }
    builder
}

/// An Owner survey of acme and octo at `EARLIER`: acme/app#1, acme/lib#2, octo/x#3.
fn earlier_owner_survey() -> CachedObservation {
    let builder = ObservationBuilder::new()
        .observed_at(EARLIER)
        .fully_read_owner("acme")
        .fully_read_owner("octo");
    let builder = read_whole(builder, "acme/app", &SourceState::Complete);
    let builder = read_whole(builder, "acme/lib", &SourceState::Complete);
    let observation = read_whole(builder, "octo/x", &SourceState::Complete)
        .pull_request(pr("acme/app", 1).author(ME))
        .pull_request(pr("acme/lib", 2).author(ME))
        .pull_request(pr("octo/x", 3).author(ME))
        .build();
    CachedObservation::fresh(observation)
}

/// A Repo Survey of acme/app that started at `at`: no Owner covered, every
/// source the Repo's own, in `state`, holding `prs`.
fn repo_survey(at: &str, state: &SourceState, prs: &[u64]) -> Observation {
    let mut builder = read_whole(ObservationBuilder::new().observed_at(at), "acme/app", state);
    for number in prs {
        builder = builder.pull_request(pr("acme/app", *number).author(ME));
    }
    builder.build()
}

fn prs(cached: &CachedObservation) -> Vec<String> {
    let mut prs: Vec<String> = cached
        .observation
        .pull_requests
        .iter()
        .map(|p| format!("{}#{}", p.repo, p.number))
        .collect();
    prs.sort();
    prs
}

fn pull_requests_of(repo: &str) -> Source {
    Source::Repo {
        repo: repo_id(repo),
        kind: SourceKind::PullRequests,
    }
}

#[test]
fn a_repo_survey_speaks_only_for_its_repo() {
    let merged = cache::merge_repo(
        Some(earlier_owner_survey()),
        repo_survey("2026-11-19T12:00:00Z", &SourceState::Complete, &[4]),
        &repo_id("acme/app"),
    );

    assert_eq!(
        prs(&merged),
        ["acme/app#4", "acme/lib#2", "octo/x#3"],
        "#1 closed since"
    );
    assert_eq!(
        merged.observation.owners,
        ["acme", "octo"],
        "the watched Owners stay"
    );
    assert_eq!(merged.observation.observed_at, now());
    // The Repo it read is as new as the Observation; everything else carries its age.
    assert_eq!(merged.carried_since(&pull_requests_of("acme/app")), None);
    assert_eq!(
        merged.carried_since(&pull_requests_of("acme/lib")),
        Some(ts(EARLIER))
    );
    assert_eq!(
        merged.carried_since(&pull_requests_of("octo/x")),
        Some(ts(EARLIER))
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(now())
    );
    assert_eq!(
        merged.read_started(&repo_id("acme/lib"), SourceKind::PullRequests),
        Some(ts(EARLIER))
    );
    assert_eq!(
        merged
            .observation
            .coverage
            .get(&pull_requests_of("acme/lib")),
        Some(&SourceState::Complete),
        "another Repo's coverage is as den held it"
    );
}

#[test]
fn a_partial_repo_survey_keeps_its_repos_earlier_facts_with_their_age() {
    let rate_limited = SourceState::Partial(Incompleteness::RateLimited { resets_at: None });
    let merged = cache::merge_repo(
        Some(earlier_owner_survey()),
        repo_survey("2026-11-19T12:00:00Z", &rate_limited, &[]),
        &repo_id("acme/app"),
    );

    assert_eq!(prs(&merged), ["acme/app#1", "acme/lib#2", "octo/x#3"]);
    assert_eq!(
        merged.carried_since(&pull_requests_of("acme/app")),
        Some(ts(EARLIER))
    );
    assert_eq!(
        merged
            .observation
            .coverage
            .get(&pull_requests_of("acme/app")),
        Some(&rate_limited),
        "the coverage is the Repo Survey's"
    );
}

#[test]
fn a_repo_survey_older_than_the_held_read_of_its_repo_changes_nothing() {
    let held = CachedObservation::fresh(repo_survey(LATER, &SourceState::Complete, &[7]));
    let merged = cache::merge_repo(
        Some(held),
        repo_survey("2026-11-19T12:00:00Z", &SourceState::Complete, &[8]),
        &repo_id("acme/app"),
    );

    assert_eq!(prs(&merged), ["acme/app#7"]);
    assert_eq!(merged.observation.observed_at, ts(LATER));
    assert_eq!(
        merged.read_started(&repo_id("acme/app"), SourceKind::PullRequests),
        Some(ts(LATER))
    );
}

#[test]
fn a_repo_survey_of_another_viewer_starts_afresh() {
    let fresh = ObservationBuilder::new().viewer("someone-else");
    let fresh = read_whole(fresh, "acme/app", &SourceState::Complete)
        .pull_request(pr("acme/app", 9))
        .build();
    let merged = cache::merge_repo(Some(earlier_owner_survey()), fresh, &repo_id("acme/app"));
    assert_eq!(prs(&merged), ["acme/app#9"]);
}

#[test]
fn fitting_the_cache_keeps_the_open_repo_of_an_unwatched_owner_and_nothing_else_of_it() {
    let builder = ObservationBuilder::new().fully_read_owner("acme");
    let builder = read_whole(builder, "acme/app", &SourceState::Complete);
    let builder = read_whole(builder, "elsewhere/tool", &SourceState::Complete);
    let observation = read_whole(builder, "elsewhere/other", &SourceState::Complete)
        .pull_request(pr("acme/app", 1))
        .pull_request(pr("elsewhere/tool", 2))
        .pull_request(pr("elsewhere/other", 3))
        .build();
    let cached = CachedObservation::fresh(observation);
    let owners = ["acme".to_owned()];

    let open = RepoId::new("Elsewhere", "Tool");
    let fitted = cached.clone().restricted_to(&owners, Some(&open));
    assert_eq!(prs(&fitted), ["acme/app#1", "elsewhere/tool#2"]);
    assert!(
        fitted
            .observation
            .coverage
            .get(&pull_requests_of("elsewhere/tool"))
            .is_some(),
        "the open Repo keeps its coverage"
    );
    assert!(
        fitted
            .observation
            .coverage
            .get(&pull_requests_of("elsewhere/other"))
            .is_none()
    );
    assert!(
        fitted
            .read_started(&repo_id("elsewhere/tool"), SourceKind::PullRequests)
            .is_some()
    );

    let unopened = cached.restricted_to(&owners, None);
    assert_eq!(prs(&unopened), ["acme/app#1"]);
}

/// Beyond the plan: an Owner survey cut off before acme/app (rate limited)
/// keeps app's facts with no Repo coverage of their own, so their age is
/// acme's. A partial Repo Survey of app keeps them still, and they keep that
/// age: the Owner's age must reach the Repo's merge, not stay behind with
/// the Owner's sources, or old facts would read as new.
#[test]
fn a_partial_repo_survey_keeps_facts_its_owner_aged_with_that_age() {
    let rate_limited = SourceState::Partial(Incompleteness::RateLimited { resets_at: None });
    let cut_off = ObservationBuilder::new()
        .observed_at("2026-11-19T11:00:00Z")
        .owner("acme")
        .owner_coverage("acme", SourceKind::Repos, rate_limited.clone())
        .fully_read_owner("octo");
    let cut_off = read_whole(cut_off, "acme/lib", &SourceState::Complete);
    let cut_off = read_whole(cut_off, "octo/x", &SourceState::Complete)
        .pull_request(pr("acme/lib", 2).author(ME))
        .pull_request(pr("octo/x", 3).author(ME))
        .build();
    let held = cache::merge(Some(earlier_owner_survey()), cut_off);
    assert_eq!(prs(&held), ["acme/app#1", "acme/lib#2", "octo/x#3"]);
    assert_eq!(
        held.carried_since(&pull_requests_of("acme/app")),
        Some(ts(EARLIER)),
        "app's facts are acme's earlier ones"
    );

    let merged = cache::merge_repo(
        Some(held),
        repo_survey("2026-11-19T12:00:00Z", &rate_limited, &[]),
        &repo_id("acme/app"),
    );

    assert_eq!(prs(&merged), ["acme/app#1", "acme/lib#2", "octo/x#3"]);
    assert_eq!(
        merged.carried_since(&pull_requests_of("acme/app")),
        Some(ts(EARLIER)),
        "kept facts keep their age"
    );
}
