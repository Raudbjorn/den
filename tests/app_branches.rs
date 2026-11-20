//! Seam 2: Branch Work Items as the App shows them, with fake ports.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Event, Key},
    cache::CachedObservation,
    domain::{NextStep, WaitingOn, WorkItemKind},
    ports::SurveyFailure,
};
use jiff::SignedDuration;
use support::{
    build::{ObservationBuilder, branch, now},
    fakes::FakeCache,
    harness::{Harness, config},
};

#[test]
fn an_unmatched_commit_author_is_shown_unattributed_in_the_list_and_the_detail() {
    let mut harness = Harness::with_owners(&["o"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("o")
        .repo("o/app")
        .branch(
            branch("o/app", "anon")
                .age_days(45)
                .ahead(4)
                .unmatched_author()
                .headline("Try a new cache layout"),
        )
        .build()));
    let view = harness.start().clone();

    let [row] = view.list.rows.as_slice() else {
        panic!("{:?}", view.list.rows)
    };
    assert_eq!(row.kind, WorkItemKind::Branch);
    assert_eq!(row.next_step, Some(NextStep::PruneOrRevive));
    assert_eq!(
        (row.repo.as_str(), row.reference.as_str()),
        ("o/app", "anon")
    );
    assert_eq!(row.title, "Try a new cache layout");
    assert_eq!(row.waiting_on, WaitingOn::Nobody);
    assert!(row.unattributed);
    assert_eq!(row.waiting_label, "unattributed");
    assert!(!row.dimmed, "unattributed work stays visible");

    let detail = view.detail.content.expect("the row is selected");
    assert_eq!(detail.next_step, "Prune or revive");
    assert_eq!(detail.waiting_on, "nobody (unattributed)");
    assert!(
        detail.why.contains("4 commits not on main"),
        "{}",
        detail.why
    );
    assert!(detail.why.contains("45 days ago"), "{}", detail.why);
    assert!(
        detail.unknowns.iter().any(|u| u.contains("unattributed")),
        "{:?}",
        detail.unknowns
    );
    assert!(
        detail.also.iter().any(|a| a.contains("not proof")),
        "{:?}",
        detail.also
    );
    assert_eq!(detail.url, "https://github.com/o/app/tree/anon");
}

#[test]
fn ticks_age_live_and_cached_branches_without_losing_scope_or_selection() {
    for from_cache in [false, true] {
        let observation = ObservationBuilder::new()
            .fully_read_owner("o")
            .fully_read_owner("other")
            .repo("o/app")
            .repo("other/app")
            .branch(branch("o/app", "threshold").age_days(30))
            .branch(branch("o/app", "recent").age_days(1))
            .branch(branch("other/app", "hidden"))
            .build();
        let mut harness = if from_cache {
            let harness = Harness::with_cache(
                Ok(config(&["o", "other"])),
                FakeCache::holding(CachedObservation::fresh(observation)),
            );
            harness.answer(Err(SurveyFailure::Interrupted {
                detail: "offline".to_owned(),
            }));
            harness
        } else {
            let harness = Harness::with_owners(&["o", "other"]);
            harness.answer(Ok(observation));
            harness
        };
        harness.start();
        harness.key(Key::Right);
        for _ in 0..harness.view.list.rows.len() {
            let selected = harness.view.list.selected.unwrap();
            if harness.view.list.rows[selected].reference == "threshold" {
                break;
            }
            harness.key(Key::Down);
        }
        let scope = harness.view.header.scope.clone();
        let selected = harness.view.list.selected.unwrap();
        assert_eq!(harness.view.list.rows[selected].reference, "threshold");
        assert_eq!(
            harness.view.list.rows[selected].next_step,
            Some(NextStep::OpenPr)
        );
        let surveys = harness.github.requests().len();
        let stores = harness.cache.stored().len();
        for elapsed in [SignedDuration::from_secs(1), SignedDuration::from_hours(24)] {
            assert!(
                harness
                    .send(Event::Tick {
                        now: now() + elapsed
                    })
                    .is_empty()
            );
            assert_eq!(harness.view.header.scope, scope);
            assert_eq!(harness.view.list.rows.len(), 2);
            let selected = harness.view.list.selected.unwrap();
            let row = &harness.view.list.rows[selected];
            assert_eq!(row.reference, "threshold");
            assert_eq!(row.next_step, Some(NextStep::PruneOrRevive));
        }
        assert!(
            harness
                .view
                .detail
                .content
                .as_ref()
                .unwrap()
                .why
                .contains("31 days ago")
        );
        assert_eq!(harness.github.requests().len(), surveys);
        assert_eq!(harness.cache.stored().len(), stores);
    }
}
