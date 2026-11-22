//! Per-Repo coverage is authoritative (spec 0003, "Coverage per Repo"): a
//! Repo with a coverage entry of its own is judged and dated by it alone, as
//! `Coverage::read_completely` already judges it. Its Owner's listing only
//! speaks for kinds the Repo has no entry of its own for.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Key},
    decide::{Decision, decide},
    domain::{
        Coverage, Incompleteness, Observation, Policies, Source, SourceKind, SourceState, Viewers,
    },
};
use support::{
    build::{ObservationBuilder, now, pr, repo_id},
    harness::Harness,
};

/// When an earlier survey was observed (two hours before `now()`).
const EARLIER: &str = "2026-11-19T10:00:00Z";

fn pagination_stopped() -> SourceState {
    SourceState::Partial(Incompleteness::PaginationStopped {
        pages_read: 1,
        reason: "a page timed out".to_owned(),
    })
}

/// acme's PR listing stopped after a page; acme/app's own PRs were read
/// completely on it, acme/big's were not reached.
fn app_read_big_not() -> Observation {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .owner_coverage("acme", SourceKind::PullRequests, pagination_stopped())
        .repo("acme/app")
        .repo("acme/big")
        .repo_coverage("acme/app", SourceKind::PullRequests, SourceState::Complete)
        .pull_request(pr("acme/app", 1))
        .pull_request(pr("acme/big", 2))
        .build()
}

fn decided(observation: &Observation, repo: &str) -> Decision {
    decide(
        observation,
        &Viewers::of(observation),
        &Policies::default(),
        now(),
    )
    .into_iter()
    .find(|d| d.item.id.repo() == Some(&repo_id(repo)))
    .unwrap_or_else(|| panic!("no Decision in {repo}"))
}

#[test]
fn a_repo_read_completely_is_judged_by_its_own_coverage_not_its_owners_listing() {
    let observation = app_read_big_not();

    let app = decided(&observation, "acme/app");
    assert!(
        app.coverage.iter().all(|e| e.state.is_complete()),
        "{:?}",
        app.coverage
    );
    assert!(
        !app.coverage.iter().any(|e| matches!(
            e.source,
            Source::Owner {
                kind: SourceKind::PullRequests,
                ..
            }
        )),
        "the Owner's listing doesn't speak for a Repo with its own entry: {:?}",
        app.coverage
    );

    let big = decided(&observation, "acme/big");
    assert!(
        big.coverage.iter().any(|e| e.state == pagination_stopped()),
        "a Repo without its own entry still carries its Owner's: {:?}",
        big.coverage
    );
}

#[test]
fn the_header_judges_each_repo_by_its_own_coverage() {
    let mut harness = Harness::with_owners(&["acme"]);
    harness.answer(Ok(app_read_big_not()));
    harness.start();

    let repos: Vec<(String, bool)> = harness
        .view
        .header
        .repos
        .iter()
        .map(|r| (r.repo.clone(), r.complete))
        .collect();
    assert_eq!(
        repos,
        [
            ("acme/app".to_owned(), true),
            ("acme/big".to_owned(), false)
        ]
    );
}

#[test]
fn a_repo_read_since_is_not_dated_by_its_owners_kept_data() {
    let mut harness = Harness::with_owners(&["acme"]);
    harness.answer(Ok(ObservationBuilder::new()
        .observed_at(EARLIER)
        .fully_read_owner("acme")
        .pull_request(pr("acme/app", 1))
        .build()));
    harness.start();
    // The refresh's Owner listing failed, so acme's earlier PR data is kept
    // from 10:00; but acme/app's own PRs were read completely just now.
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .owner_coverage(
            "acme",
            SourceKind::PullRequests,
            SourceState::Failed {
                error: "gh timed out".to_owned(),
            },
        )
        .repo_coverage("acme/app", SourceKind::PullRequests, SourceState::Complete)
        .pull_request(pr("acme/app", 1))
        .build()));
    let effects = harness.key(Key::Char('r'));
    let survey = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .expect("r asks for a survey")
        .clone();
    harness.perform(&survey);

    let detail = harness.view.detail.content.clone().expect("selected");
    assert_eq!(detail.reference, "acme/app#1");
    assert_eq!(detail.observed, "12:00 UTC, just now");
}

#[test]
fn an_owner_survey_records_each_listed_repos_sources_as_its_own() {
    let owner = |kind| Source::Owner {
        owner: "acme".to_owned(),
        kind,
    };
    let repo = |name: &str, kind| Source::Repo {
        repo: repo_id(name),
        kind,
    };
    let mut coverage = Coverage::default();
    coverage.record(owner(SourceKind::Repos), SourceState::Complete);
    coverage.record(owner(SourceKind::PullRequests), pagination_stopped());
    coverage.record(owner(SourceKind::Issues), SourceState::Complete);
    coverage.record(owner(SourceKind::Branches), SourceState::Complete);
    coverage.record(owner(SourceKind::Runs), SourceState::NotYetFetched);
    // acme/app's PRs all came on the page den read.
    coverage.record(
        repo("acme/app", SourceKind::PullRequests),
        SourceState::Complete,
    );

    coverage.split_by_repo(
        "acme",
        &[
            repo_id("acme/app"),
            repo_id("acme/big"),
            repo_id("octo/elsewhere"),
        ],
    );

    let state = |source: Source| coverage.get(&source).cloned();
    for name in ["acme/app", "acme/big"] {
        assert_eq!(
            state(repo(name, SourceKind::Repos)),
            Some(SourceState::Complete),
            "{name}"
        );
        assert_eq!(
            state(repo(name, SourceKind::Issues)),
            Some(SourceState::Complete),
            "{name}"
        );
        assert_eq!(
            state(repo(name, SourceKind::Branches)),
            Some(SourceState::Complete),
            "{name}"
        );
        assert_eq!(
            state(repo(name, SourceKind::Runs)),
            Some(SourceState::NotYetFetched),
            "{name}"
        );
    }
    assert_eq!(
        state(repo("acme/app", SourceKind::PullRequests)),
        Some(SourceState::Complete),
        "a Repo's own pages read completely"
    );
    assert_eq!(
        state(repo("acme/big", SourceKind::PullRequests)),
        Some(pagination_stopped()),
        "a Repo the listing didn't finish carries the Owner's reason"
    );
    assert!(
        !coverage
            .entries()
            .iter()
            .any(|e| matches!(&e.source, Source::Repo { repo, .. } if repo.owner == "octo")),
        "another Owner's Repo is not this Owner's to split"
    );
}
