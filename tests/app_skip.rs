//! Seam 2, per-Repo `skip` (spec 0003): Work Items whose only Next Step their
//! Repo doesn't track leave the list, an empty list says what it left out,
//! and a selection that stops being tracked gives way to the next row.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Event, Key},
    config,
    domain::Observation,
};
use std::path::Path;
use support::{
    build::{ME, ObservationBuilder, now, pr, run},
    harness::Harness,
};

const CONFIG: &str = "owners = [\"acme\"]\n\n[repos.\"acme/app\"]\nskip = [\"Fix CI\"]\n";

fn started(observation: Observation) -> Harness {
    let mut harness = Harness::new(config::parse(
        Path::new("/home/me/.config/den/config.toml"),
        CONFIG,
    ));
    harness.answer(Ok(observation));
    harness.start();
    harness
}

fn acme() -> ObservationBuilder {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/app")
}

fn row(repo: &str, reference: &str, step: &str) -> (String, String, String) {
    (repo.to_owned(), reference.to_owned(), step.to_owned())
}

#[test]
fn work_items_whose_only_next_step_is_not_tracked_leave_the_list() {
    let harness = started(
        acme()
            .run(run("acme/app", "ci.yml", 1).failed())
            .pull_request(pr("acme/app", 1).author(ME).draft())
            .build(),
    );
    assert_eq!(harness.rows(), [row("acme/app", "#1", "Finish draft")]);
}

#[test]
fn an_empty_list_says_what_it_does_not_track() {
    let harness = started(acme().run(run("acme/app", "ci.yml", 1).failed()).build());
    assert!(harness.view.list.rows.is_empty());
    let empty = harness.view.list.empty.clone().expect("an empty state");
    assert_eq!(empty.title, "Nothing needs you.");
    assert!(
        empty
            .detail
            .ends_with("Not tracked here (skip in config.toml): Fix CI, on 1 Work Item."),
        "{empty:?}"
    );
}

#[test]
fn a_narrowed_scope_does_not_count_its_own_untracked_work_items_as_elsewhere() {
    let mut harness = started(acme().run(run("acme/app", "ci.yml", 1).failed()).build());
    harness.key(Key::Right);
    assert_eq!(harness.view.header.scope, "Owner · acme");
    assert!(harness.view.list.rows.is_empty());
    let empty = harness.view.list.empty.clone().expect("an empty state");
    assert!(
        empty
            .detail
            .starts_with("Nothing in any other Scope either"),
        "the only Work Item is in this Scope, just not tracked: {empty:?}"
    );
    assert!(
        empty
            .detail
            .ends_with("Not tracked here (skip in config.toml): Fix CI, on 1 Work Item."),
        "{empty:?}"
    );
}

#[test]
fn a_selected_work_item_that_stops_being_tracked_gives_way() {
    let mut harness = started(
        acme()
            .pull_request(
                pr("acme/app", 5)
                    .author(ME)
                    .draft()
                    .updated("2026-11-18T12:00:00Z"),
            )
            .pull_request(
                pr("acme/app", 6)
                    .author(ME)
                    .draft()
                    .updated("2026-11-17T12:00:00Z"),
            )
            .build(),
    );
    assert_eq!(
        harness.rows(),
        [
            row("acme/app", "#5", "Finish draft"),
            row("acme/app", "#6", "Finish draft")
        ]
    );
    assert_eq!(harness.view.list.selected, Some(0));

    // #5 is no longer a draft; only its failing checks remain, and Fix CI isn't tracked here.
    harness.answer(Ok(acme()
        .pull_request(
            pr("acme/app", 5)
                .author(ME)
                .failing_required("ci")
                .updated("2026-11-18T13:00:00Z"),
        )
        .pull_request(
            pr("acme/app", 6)
                .author(ME)
                .draft()
                .updated("2026-11-17T12:00:00Z"),
        )
        .build()));
    harness.settle(Event::Started { now: now() });

    assert_eq!(harness.rows(), [row("acme/app", "#6", "Finish draft")]);
    assert_eq!(harness.view.list.selected, Some(0));
}
