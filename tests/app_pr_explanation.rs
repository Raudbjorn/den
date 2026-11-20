//! Seam 2: the detail pane shows a PR's full Explanation: the winning rule,
//! the other conditions (failing checks labelled required or advisory), the
//! facts den couldn't establish, why it is Waiting On whoever it is, and when
//! the facts were observed. Asserts on the ViewModel only.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::Key,
    domain::{Incompleteness, NextStep, SourceKind, SourceState, WaitingOn},
};
use support::{
    build::{ME, ObservationBuilder, pr},
    harness::Harness,
};

#[test]
fn the_detail_pane_shows_the_winning_rule_other_conditions_and_waiting_on_reason() {
    let mut harness = Harness::with_owners(&["octo"]);
    harness.answer(Ok(ObservationBuilder::new()
        .team("octo", "core")
        .owner("octo")
        .observed_at("2026-11-19T11:56:00Z")
        .pull_request(
            pr("octo/app", 7)
                .author(ME)
                .conflicting()
                .failing_required("CI / build")
                .failing_advisory("lint")
                .review_requested_from_team("octo", "core"),
        )
        .build()));
    let view = harness.start().clone();

    let row = &view.list.rows[0];
    assert_eq!(row.next_step, Some(NextStep::ResolveConflicts));
    assert_eq!(row.waiting_on, WaitingOn::Me);

    let detail = view.detail.content.expect("the PR is selected");
    assert_eq!(detail.next_step, "Resolve conflicts");
    assert_eq!(detail.why, "conflicts with main");
    assert_eq!(
        detail.also,
        [
            "2 failing checks, 1 required and 1 advisory: CI / build (required), lint (advisory)",
            "review requested from your team octo/core",
        ]
    );
    assert!(detail.unknowns.is_empty(), "{:?}", detail.unknowns);
    assert_eq!(detail.waiting_on, "you");
    assert_eq!(detail.waiting_reason, "you are the PR author (me)");
    assert_eq!(detail.observed, "11:56 UTC, 4 min ago");
}

#[test]
fn a_pr_with_no_next_step_says_why_and_lists_what_den_could_not_establish() {
    let mut harness = Harness::with_owners(&["octo"]);
    let lost = SourceState::Partial(Incompleteness::FollowUpLost {
        what: "checks and reviews of #3".to_owned(),
        reason: "failed: timed out after 30s".to_owned(),
    });
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .repo_coverage("octo/app", SourceKind::PullRequests, lost)
        .pull_request(
            pr("octo/app", 3)
                .author("alice")
                .approved()
                .mergeability_unknown()
                .details_unread(),
        )
        .build()));
    let view = harness.start().clone();

    let detail = view.detail.content.expect("the PR is selected");
    assert_eq!(detail.next_step, "none");
    assert!(
        detail.why.starts_with("no rule matched: "),
        "{}",
        detail.why
    );
    assert!(
        detail.why.contains("checks could not be read"),
        "{}",
        detail.why
    );
    assert!(
        detail
            .why
            .contains("waiting for GitHub to compute mergeability"),
        "{}",
        detail.why
    );
    for unknown in [
        "mergeability not computed yet by GitHub",
        "checks could not be read",
        "review threads could not be read",
        "review requests could not be read",
    ] {
        assert!(
            detail.unknowns.iter().any(|u| u == unknown),
            "{unknown:?} in {:?}",
            detail.unknowns
        );
    }
    assert_eq!(detail.waiting_on, "nobody");
    assert!(!detail.waiting_reason.is_empty());
    assert_eq!(
        detail.coverage,
        [
            "Pull Requests of octo/app: partial: checks and reviews of #3 could not be read: failed: timed out after 30s"
        ]
    );
    assert!(view.list.rows[0].incomplete);
}

#[test]
fn moving_the_selection_shows_each_prs_own_explanation() {
    let mut harness = Harness::with_owners(&["octo"]);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .pull_request(
            pr("octo/app", 1)
                .author(ME)
                .approved()
                .passing("CI / build")
                .updated("2026-11-19T11:00:00Z"),
        )
        .pull_request(
            pr("octo/app", 2)
                .author(ME)
                .failing_advisory("coverage")
                .updated("2026-11-19T10:00:00Z"),
        )
        .build()));
    harness.start();
    let steps: Vec<(String, String)> = [None, Some(Key::Down)]
        .into_iter()
        .map(|key| {
            if let Some(key) = key {
                harness.key(key);
            }
            let detail = harness.view.detail.content.clone().expect("selected");
            (detail.next_step, detail.why)
        })
        .collect();
    assert_eq!(
        steps,
        [
            (
                "Merge".to_owned(),
                "approved, checks passing and mergeable into main".to_owned()
            ),
            (
                "Fix CI".to_owned(),
                "1 failing check, advisory only: coverage (advisory)".to_owned()
            ),
        ]
    );
}
