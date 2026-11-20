//! Seam 2: Branch Work Items as the App shows them, with fake ports.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::domain::{NextStep, WaitingOn, WorkItemKind};
use support::{
    build::{ObservationBuilder, branch},
    harness::Harness,
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
