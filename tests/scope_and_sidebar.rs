//! Seam 2: the sidebar, Scope switching, the list's order and a selection that
//! survives refreshes and Scope changes. Asserts on the ViewModel only.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Event, Key,
        view_model::{SidebarEntry, SidebarRow, ViewModel},
    },
    domain::{Observation, SourceKind, SourceState, WaitingOn},
    ordering::Scope,
};
use support::{
    build::{ME, ObservationBuilder, now, pr, repo_id},
    harness::Harness,
};

/// Two Owners: `octo` with a plain Repo, a fork and an archived Repo; `beta`
/// with one Repo that has no Work Items.
fn two_owners() -> Observation {
    ObservationBuilder::new()
        .owner("octo")
        .owner("beta")
        .repo("octo/app")
        .fork("octo/fork", Some("upstream/fork"))
        .archived_repo("octo/old")
        .repo("beta/quiet")
        .pull_request(pr("octo/app", 1).draft().author(ME).title("mine"))
        .pull_request(pr("octo/app", 2).title("no step"))
        .pull_request(pr("octo/fork", 3).draft().author(ME).title("fork draft"))
        .pull_request(pr("octo/old", 4).draft().author("alice").title("old"))
        .build()
}

fn started(observation: Observation) -> Harness {
    let mut harness = Harness::with_owners(&["octo", "beta"]);
    harness.answer(Ok(observation));
    harness.start();
    harness
}

/// A refresh: den surveys again and the given answer arrives.
fn refresh(harness: &mut Harness, observation: Observation) {
    harness.answer(Ok(observation));
    harness.settle(Event::Started { now: now() });
}

fn row<'a>(view: &'a ViewModel, scope: &Scope) -> &'a SidebarRow {
    view.sidebar
        .rows
        .iter()
        .find(|r| &r.scope == scope)
        .unwrap_or_else(|| panic!("no sidebar row for {scope:?}: {:?}", view.sidebar.rows))
}

fn references(harness: &Harness) -> Vec<String> {
    harness
        .rows()
        .into_iter()
        .map(|(repo, reference, _)| format!("{repo}{reference}"))
        .collect()
}

fn selected(harness: &Harness) -> Option<String> {
    let list = &harness.view.list;
    list.selected
        .map(|i| format!("{}{}", list.rows[i].repo, list.rows[i].reference))
}

fn segments(view: &ViewModel) -> Vec<(String, bool)> {
    view.header
        .scopes
        .iter()
        .map(|s| (s.label.clone(), s.active))
        .collect()
}

#[test]
fn the_sidebar_lists_every_repo_by_owner_with_counts_and_fork_and_archived_marks() {
    let harness = started(two_owners());
    let view = &harness.view;

    let rows: Vec<(SidebarEntry, &str, usize)> = view
        .sidebar
        .rows
        .iter()
        .map(|r| (r.entry.clone(), r.label.as_str(), r.count))
        .collect();
    assert_eq!(
        rows,
        [
            (SidebarEntry::AllOwners, "All Owners", 3),
            (SidebarEntry::Owner, "octo", 3),
            (SidebarEntry::Repo, "app", 1),
            (SidebarEntry::Repo, "fork", 1),
            (SidebarEntry::Repo, "old", 1),
            (SidebarEntry::Owner, "beta", 0),
            (SidebarEntry::Repo, "quiet", 0),
        ]
    );
    let fork = row(view, &Scope::Repo(repo_id("octo/fork")));
    assert!(fork.fork && !fork.archived);
    let old = row(view, &Scope::Repo(repo_id("octo/old")));
    assert!(old.archived && !old.fork);
    let app = row(view, &Scope::Repo(repo_id("octo/app")));
    assert!(!app.fork && !app.archived);
    assert!(row(view, &Scope::AllOwners).current);
    assert!(!view.sidebar.focused);
}

#[test]
fn a_repo_known_only_from_its_work_items_still_gets_a_sidebar_row() {
    // The Repo list was read incompletely and missed `octo/hidden`.
    let harness = started(
        ObservationBuilder::new()
            .owner("octo")
            .owner("beta")
            .owner_coverage(
                "octo",
                SourceKind::Repos,
                SourceState::Failed {
                    error: "timeout".to_owned(),
                },
            )
            .pull_request(pr("octo/hidden", 9).draft().author(ME))
            .build(),
    );
    let hidden = row(&harness.view, &Scope::Repo(repo_id("octo/hidden")));
    assert_eq!(hidden.count, 1);
}

#[test]
fn arrow_keys_step_the_scope_through_owners_filtering_the_list_and_naming_it_in_the_header() {
    let mut harness = started(two_owners());
    assert_eq!(references(&harness).len(), 4);
    assert!(harness.view.header.scope.starts_with("All Owners"));
    assert_eq!(
        segments(&harness.view),
        [
            ("All Owners".to_owned(), true),
            ("octo".to_owned(), false),
            ("beta".to_owned(), false),
        ]
    );

    harness.key(Key::Right);
    assert_eq!(harness.view.header.scope, "Owner · octo");
    assert_eq!(references(&harness).len(), 4);
    assert_eq!(segments(&harness.view)[1], ("octo".to_owned(), true));
    assert!(row(&harness.view, &Scope::Owner("octo".to_owned())).current);

    harness.key(Key::Right);
    assert_eq!(harness.view.header.scope, "Owner · beta");
    assert!(references(&harness).is_empty());
    let empty = harness.view.list.empty.clone().expect("an empty state");
    assert!(
        !empty.title.contains("Nothing needs you"),
        "a narrowed Scope must not claim nothing needs you: {empty:?}"
    );
    assert!(empty.detail.contains("4 Work Items"), "{empty:?}");

    harness.key(Key::Right);
    assert!(harness.view.header.scope.starts_with("All Owners"), "wraps");
    harness.key(Key::Left);
    assert_eq!(harness.view.header.scope, "Owner · beta", "wraps back");
}

#[test]
fn the_sidebar_scopes_to_one_repo_from_the_keyboard() {
    let mut harness = started(two_owners());
    harness.key(Key::Tab);
    assert!(harness.view.sidebar.focused);
    // The cursor starts on the current Scope (All Owners): down to octo, then fork.
    harness.key(Key::Down);
    harness.key(Key::Down);
    harness.key(Key::Down);
    let cursor = harness.view.sidebar.cursor.expect("a cursor");
    assert_eq!(harness.view.sidebar.rows[cursor].label, "fork");
    assert!(
        harness.view.header.scope.starts_with("All Owners"),
        "moving the cursor alone does not change the Scope"
    );
    assert!(
        harness
            .view
            .footer
            .hints
            .iter()
            .any(|h| h.label == "scope to"),
        "{:?}",
        harness.view.footer.hints
    );

    harness.key(Key::Enter);
    assert!(!harness.view.sidebar.focused, "Enter returns to the list");
    assert_eq!(
        harness.view.header.scope,
        "Repo · octo/fork · fork of upstream/fork"
    );
    assert_eq!(references(&harness), ["octo/fork#3"]);
    assert_eq!(
        segments(&harness.view).last(),
        Some(&("fork".to_owned(), true))
    );

    // ← widens a Repo Scope to its Owner.
    harness.key(Key::Left);
    assert_eq!(harness.view.header.scope, "Owner · octo");

    // Esc leaves the sidebar without changing the Scope.
    harness.key(Key::Tab);
    harness.key(Key::Down);
    harness.key(Key::Esc);
    assert!(!harness.view.sidebar.focused);
    assert_eq!(harness.view.header.scope, "Owner · octo");
}

#[test]
fn the_archived_repo_scope_says_so_in_the_header() {
    let mut harness = started(two_owners());
    harness.key(Key::Tab);
    for _ in 0..4 {
        harness.key(Key::Down);
    }
    harness.key(Key::Enter);
    assert_eq!(harness.view.header.scope, "Repo · octo/old · archived");
}

#[test]
fn the_list_is_ordered_by_waiting_on_then_updated_and_someone_elses_items_are_dimmed_and_labelled()
{
    let harness = started(
        ObservationBuilder::new()
            .owner("octo")
            .owner("beta")
            .pull_request(
                pr("octo/a", 1)
                    .draft()
                    .author("alice")
                    .updated("2026-11-19T11:00:00Z"),
            )
            .pull_request(pr("octo/a", 2).updated("2026-11-19T10:00:00Z"))
            .pull_request(pr("octo/a", 3).draft().no_author())
            .pull_request(
                pr("octo/a", 4)
                    .draft()
                    .author(ME)
                    .updated("2026-11-01T00:00:00Z"),
            )
            .pull_request(
                pr("beta/b", 5)
                    .draft()
                    .author(ME)
                    .updated("2026-11-10T00:00:00Z"),
            )
            .build(),
    );
    // You (newest first) › nobody (a step before no step) › someone else.
    assert_eq!(
        references(&harness),
        ["beta/b#5", "octo/a#4", "octo/a#3", "octo/a#2", "octo/a#1"]
    );
    let rows = &harness.view.list.rows;
    let waiting: Vec<WaitingOn> = rows.iter().map(|r| r.waiting_on).collect();
    assert_eq!(
        waiting,
        [
            WaitingOn::Me,
            WaitingOn::Me,
            WaitingOn::Nobody,
            WaitingOn::Nobody,
            WaitingOn::SomeoneElse
        ]
    );
    let someone = &rows[4];
    assert!(someone.dimmed);
    // Without colour: the words say who it is waiting on.
    assert_eq!(someone.waiting_label, "someone else");
    assert!(rows[..4].iter().all(|r| !r.dimmed));
}

#[test]
fn selection_survives_a_scope_change_when_the_item_stays_in_scope() {
    let mut harness = started(two_owners());
    // Select octo/fork#3.
    while selected(&harness).as_deref() != Some("octo/fork#3") {
        harness.key(Key::Down);
    }
    harness.key(Key::Right); // Owner octo: still in Scope.
    assert_eq!(selected(&harness).as_deref(), Some("octo/fork#3"));

    harness.key(Key::Right); // Owner beta: not in Scope, nothing to select.
    assert_eq!(selected(&harness), None);
    assert!(harness.view.detail.content.is_none());

    harness.key(Key::Right); // All Owners: the first item.
    assert_eq!(selected(&harness), references(&harness).first().cloned());
}

#[test]
fn selection_and_scope_survive_a_refresh() {
    let mut harness = started(two_owners());
    harness.key(Key::Right); // Owner octo.
    harness.key(Key::Down);
    let before = selected(&harness).expect("a selection");

    // The refresh brings a newer item that sorts first.
    let refreshed = ObservationBuilder::new()
        .owner("octo")
        .owner("beta")
        .repo("octo/app")
        .fork("octo/fork", Some("upstream/fork"))
        .archived_repo("octo/old")
        .repo("beta/quiet")
        .pull_request(
            pr("octo/app", 10)
                .draft()
                .author(ME)
                .updated("2026-11-19T11:59:00Z"),
        )
        .pull_request(pr("octo/app", 1).draft().author(ME).title("mine"))
        .pull_request(pr("octo/app", 2).title("no step"))
        .pull_request(pr("octo/fork", 3).draft().author(ME).title("fork draft"))
        .pull_request(pr("octo/old", 4).draft().author("alice").title("old"))
        .pull_request(pr("beta/quiet", 5).draft().author(ME))
        .build();
    refresh(&mut harness, refreshed);

    assert_eq!(harness.view.header.scope, "Owner · octo");
    assert_eq!(
        references(&harness).first().map(String::as_str),
        Some("octo/app#10")
    );
    assert!(!references(&harness).iter().any(|r| r.starts_with("beta/")));
    assert_eq!(selected(&harness), Some(before));
    assert_eq!(row(&harness.view, &Scope::AllOwners).count, 5);
}

#[test]
fn the_sidebar_cursor_follows_its_row_across_a_refresh() {
    let mut harness = started(two_owners());
    harness.key(Key::Tab);
    for _ in 0..3 {
        harness.key(Key::Down);
    }
    let cursor_label = |h: &Harness| {
        h.view.sidebar.rows[h.view.sidebar.cursor.expect("a cursor")]
            .label
            .clone()
    };
    assert_eq!(cursor_label(&harness), "fork");
    // A new Repo sorts before `fork`, shifting the rows.
    refresh(
        &mut harness,
        ObservationBuilder::new()
            .owner("octo")
            .owner("beta")
            .repo("octo/aardvark")
            .repo("octo/app")
            .fork("octo/fork", None)
            .build(),
    );
    assert!(harness.view.sidebar.focused);
    assert_eq!(cursor_label(&harness), "fork");
}

#[test]
fn enter_scopes_to_the_row_shown_under_the_cursor_when_a_refresh_removed_its_repo() {
    let mut harness = started(two_owners());
    harness.key(Key::Right); // Owner octo: the Scope's row stays after the refresh.
    harness.key(Key::Tab);
    harness.key(Key::Down); // From octo down to app.
    let cursor = harness.view.sidebar.cursor.expect("a cursor");
    assert_eq!(harness.view.sidebar.rows[cursor].label, "app");

    // `octo/app` is gone: neither listed nor carrying a Work Item.
    refresh(
        &mut harness,
        ObservationBuilder::new()
            .owner("octo")
            .owner("beta")
            .fork("octo/fork", Some("upstream/fork"))
            .pull_request(pr("octo/fork", 3).draft().author(ME))
            .build(),
    );
    let cursor = harness.view.sidebar.cursor.expect("a cursor");
    assert_eq!(
        harness.view.sidebar.rows[cursor].label, "octo",
        "the cursor falls back to the Scope's row"
    );

    harness.key(Key::Enter);
    assert_eq!(
        harness.view.header.scope, "Owner · octo",
        "Enter scopes to the row drawn under the cursor, not the removed Repo"
    );
    assert_eq!(references(&harness), ["octo/fork#3"]);
}

#[test]
fn a_refresh_that_changes_a_repos_casing_keeps_the_scope_cursor_and_selection() {
    let mut harness = started(two_owners());
    harness.key(Key::Tab);
    for _ in 0..3 {
        harness.key(Key::Down); // All Owners → octo → app → fork.
    }
    harness.key(Key::Enter); // Scope: Repo octo/fork.
    harness.key(Key::Tab);
    assert_eq!(selected(&harness).as_deref(), Some("octo/fork#3"));

    // GitHub now spells the Owner and the Repo with capitals.
    refresh(
        &mut harness,
        ObservationBuilder::new()
            .owner("octo")
            .owner("beta")
            .repo("Octo/app")
            .fork("Octo/Fork", Some("upstream/fork"))
            .pull_request(pr("Octo/Fork", 3).draft().author(ME))
            .pull_request(pr("Octo/Fork", 7).draft().author(ME))
            .build(),
    );
    let fork = row(&harness.view, &Scope::Repo(repo_id("Octo/Fork")));
    assert!(fork.current, "the current-Scope mark stays on the Repo");
    let cursor = harness.view.sidebar.cursor.expect("a cursor");
    assert_eq!(harness.view.sidebar.rows[cursor].label, "Fork");
    assert_eq!(
        selected(&harness).as_deref(),
        Some("Octo/Fork#3"),
        "the same Pull Request stays selected"
    );
}

#[test]
fn a_failed_refresh_shows_its_details_in_an_empty_narrowed_scope() {
    let mut harness = started(two_owners());
    harness.key(Key::Left); // Owner beta: no Work Items.
    assert_eq!(harness.view.header.scope, "Owner · beta");
    harness.answer(Err(den::ports::SurveyFailure::NotLoggedIn {
        detail: "token expired".to_owned(),
    }));
    harness.settle(Event::Started { now: now() });

    let empty = harness.view.list.empty.clone().expect("an empty state");
    assert_eq!(empty.title, "gh is not logged in");
    assert!(
        empty.detail.contains("gh auth login") && empty.detail.contains("token expired"),
        "the recovery details are shown: {empty:?}"
    );
}

#[test]
fn an_empty_narrowed_scope_does_not_rule_out_other_scopes_when_coverage_is_incomplete() {
    // Nothing anywhere, and beta's Pull Requests failed to load.
    let mut harness = started(
        ObservationBuilder::new()
            .owner("octo")
            .owner("beta")
            .repo("octo/app")
            .owner_coverage(
                "beta",
                SourceKind::PullRequests,
                SourceState::Failed {
                    error: "timeout".to_owned(),
                },
            )
            .build(),
    );
    harness.key(Key::Right); // Owner octo.
    let empty = harness.view.list.empty.clone().expect("an empty state");
    assert!(
        !empty.detail.contains("Nothing in any other Scope"),
        "zero known Work Items elsewhere is not nothing elsewhere: {empty:?}"
    );
    assert!(empty.detail.contains("incomplete"), "{empty:?}");
}
