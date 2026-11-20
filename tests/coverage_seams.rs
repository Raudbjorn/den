//! Coverage across the wave-2 seams (integration of #4–#9).
//!
//! - An Owner's Repo list read only partly makes every Work Item kind of that
//!   Owner partial, with the reason (seam 1: carried onto each Decision; seam 2:
//!   rows, Detail, header, sidebar, empty states and the cache merge).
//! - The sidebar marks every row whose Scope was not read completely.
//! - A narrowed Scope's empty state judges that Scope's own coverage.
//! - The Detail pane dates kept data by when it was observed, not by the refresh.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Key, ViewModel, view_model::SidebarRow},
    decide::{Decision, decide},
    domain::{Incompleteness, Observation, Source, SourceKind, SourceState},
    ordering::Scope,
};
use support::{
    build::{ObservationBuilder, branch, issue, now, pr, repo_id, run},
    harness::Harness,
};

/// When an earlier survey was observed (two hours before `now()`).
const EARLIER: &str = "2026-11-19T10:00:00Z";

fn pagination_stopped() -> SourceState {
    SourceState::Partial(Incompleteness::PaginationStopped {
        pages_read: 3,
        reason: "a page timed out".to_owned(),
    })
}

fn repo_list_incomplete(owner: &str) -> SourceState {
    SourceState::Partial(Incompleteness::RepoListIncomplete {
        owner: owner.to_owned(),
    })
}

fn decisions(observation: &Observation) -> Vec<Decision> {
    decide(observation, &observation.viewer, now())
}

/// The state of `kind` for `owner` that a Decision carries, if any.
fn carried_state(decision: &Decision, owner: &str, kind: SourceKind) -> Option<SourceState> {
    let source = Source::Owner {
        owner: owner.to_owned(),
        kind,
    };
    decision
        .coverage
        .iter()
        .find(|e| e.source == source)
        .map(|e| e.state.clone())
}

/// alpha's Repo list stopped paginating; beta was read completely. Each has a
/// PR, alpha also an Issue, a failing Run and a Branch with commits ahead.
fn alpha_repo_list_partial() -> Observation {
    ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .owner_coverage("alpha", SourceKind::Repos, pagination_stopped())
        .repo("alpha/app")
        .repo("beta/lib")
        .pull_request(pr("alpha/app", 1))
        .issue(issue("alpha/app", 2))
        .run(run("alpha/app", "ci.yml", 3).failed())
        .branch(branch("alpha/app", "feature").ahead(2).age_days(1))
        .pull_request(pr("beta/lib", 4))
        .build()
}

fn sidebar_row<'a>(view: &'a ViewModel, scope: &Scope) -> &'a SidebarRow {
    view.sidebar
        .rows
        .iter()
        .find(|r| &r.scope == scope)
        .unwrap_or_else(|| panic!("no sidebar row for {scope}"))
}

fn empty_title(view: &ViewModel) -> String {
    view.list
        .empty
        .as_ref()
        .map(|e| e.title.clone())
        .unwrap_or_default()
}

// ---- seam 1 -------------------------------------------------------------

#[test]
fn a_partly_read_repo_list_is_carried_onto_every_kind_of_that_owners_work_items() {
    let decided = decisions(&alpha_repo_list_partial());

    let alpha: Vec<&Decision> = decided
        .iter()
        .filter(|d| d.item.id.repo().owner == "alpha")
        .collect();
    assert_eq!(alpha.len(), 4, "a PR, an Issue, a Run and a Branch");
    for decision in alpha {
        let kind = SourceKind::for_work_item(decision.item.id.kind());
        assert_eq!(
            carried_state(decision, "alpha", kind),
            Some(repo_list_incomplete("alpha")),
            "{} carries alpha's partial Repo list",
            decision.item.id
        );
    }
    let beta = decided
        .iter()
        .find(|d| d.item.id.repo().owner == "beta")
        .expect("beta's PR");
    assert!(
        beta.coverage.iter().all(|e| e.state.is_complete()),
        "beta's Repo list was complete: {:?}",
        beta.coverage
    );
}

#[test]
fn a_kinds_own_reason_wins_over_the_repo_list_whichever_is_recorded_first() {
    let rate_limited = SourceState::Partial(Incompleteness::RateLimited { resets_at: None });
    let failed = SourceState::Failed {
        error: "gh timed out".to_owned(),
    };
    // Repos first, then the kind; and the kind first, then Repos.
    let repos_first = ObservationBuilder::new()
        .owner_coverage("alpha", SourceKind::Repos, pagination_stopped())
        .owner_coverage("alpha", SourceKind::PullRequests, rate_limited.clone())
        .owner_coverage("alpha", SourceKind::Issues, failed.clone())
        .pull_request(pr("alpha/app", 1))
        .issue(issue("alpha/app", 2))
        .build();
    let kinds_first = ObservationBuilder::new()
        .owner_coverage("alpha", SourceKind::PullRequests, rate_limited.clone())
        .owner_coverage("alpha", SourceKind::Issues, failed.clone())
        .owner_coverage("alpha", SourceKind::Repos, pagination_stopped())
        .pull_request(pr("alpha/app", 1))
        .issue(issue("alpha/app", 2))
        .build();

    for observation in [repos_first, kinds_first] {
        let decided = decisions(&observation);
        let state_of = |kind| {
            decided
                .iter()
                .find_map(|d| carried_state(d, "alpha", kind))
                .expect("carried")
        };
        assert_eq!(state_of(SourceKind::PullRequests), rate_limited);
        assert_eq!(state_of(SourceKind::Issues), failed);
    }
}

#[test]
fn a_failed_repo_list_makes_kinds_partial_and_an_unfetched_one_leaves_them_unfetched() {
    let failed = ObservationBuilder::new()
        .owner_coverage(
            "alpha",
            SourceKind::Repos,
            SourceState::Failed {
                error: "HTTP 502".to_owned(),
            },
        )
        .owner_coverage("alpha", SourceKind::PullRequests, SourceState::Complete)
        .pull_request(pr("alpha/app", 1))
        .build();
    assert_eq!(
        carried_state(&decisions(&failed)[0], "alpha", SourceKind::PullRequests),
        Some(repo_list_incomplete("alpha"))
    );

    let unfetched = ObservationBuilder::new()
        .owner_coverage("alpha", SourceKind::Repos, SourceState::NotYetFetched)
        .pull_request(pr("alpha/app", 1))
        .build();
    assert_eq!(
        carried_state(&decisions(&unfetched)[0], "alpha", SourceKind::PullRequests),
        Some(SourceState::NotYetFetched)
    );
}

// ---- seam 2 -------------------------------------------------------------

#[test]
fn a_partly_read_repo_list_flags_the_owners_rows_and_says_why_in_detail_header_and_sidebar() {
    let mut harness = Harness::with_owners(&["alpha", "beta"]);
    harness.answer(Ok(alpha_repo_list_partial()));
    let view = harness.start().clone();

    for row in &view.list.rows {
        assert_eq!(
            row.incomplete,
            row.repo.starts_with("alpha/"),
            "{}{}",
            row.repo,
            row.reference
        );
    }

    assert!(!view.header.coverage_complete);
    for kind in [
        "Pull Requests of alpha",
        "Issues of alpha",
        "Branches of alpha",
        "Runs of alpha",
    ] {
        assert!(
            view.header.coverage.contains(kind),
            "{kind}: {}",
            view.header.coverage
        );
    }
    assert!(
        view.header
            .coverage
            .contains("Repo list of alpha was only partly read"),
        "{}",
        view.header.coverage
    );

    assert!(sidebar_row(&view, &Scope::AllOwners).incomplete);
    assert!(sidebar_row(&view, &Scope::Owner("alpha".into())).incomplete);
    assert!(sidebar_row(&view, &Scope::Repo(repo_id("alpha/app"))).incomplete);
    assert!(!sidebar_row(&view, &Scope::Owner("beta".into())).incomplete);
    assert!(!sidebar_row(&view, &Scope::Repo(repo_id("beta/lib"))).incomplete);

    let alpha_index = view
        .list
        .rows
        .iter()
        .position(|r| r.repo == "alpha/app")
        .expect("an alpha row");
    for _ in 0..alpha_index {
        harness.key(Key::Down);
    }
    let detail = harness.view.detail.content.clone().expect("selected");
    assert!(detail.reference.starts_with("alpha/app"));
    assert!(
        detail
            .coverage
            .iter()
            .any(|line| line.contains("Repo list of alpha was only partly read")),
        "{:?}",
        detail.coverage
    );
}

#[test]
fn an_empty_survey_with_a_partly_read_repo_list_never_says_nothing_needs_you() {
    let mut harness = Harness::with_owners(&["alpha"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .owner_coverage("alpha", SourceKind::Repos, pagination_stopped())
        .build()));
    let view = harness.start().clone();

    assert!(view.list.rows.is_empty());
    let title = empty_title(&view);
    assert_ne!(title, "Nothing needs you.");
    assert!(title.contains("incomplete"), "{title}");
    assert!(!view.header.coverage_complete);
    assert!(sidebar_row(&view, &Scope::AllOwners).incomplete);
}

#[test]
fn the_sidebar_marks_exactly_the_rows_whose_scope_was_not_read_completely() {
    let mut harness = Harness::with_owners(&["alpha", "beta"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .repo("alpha/app")
        .repo("alpha/big")
        .repo("beta/lib")
        .repo_coverage("alpha/big", SourceKind::PullRequests, pagination_stopped())
        .build()));
    let view = harness.start().clone();

    let marked: Vec<(String, bool)> = view
        .sidebar
        .rows
        .iter()
        .map(|r| (r.scope.to_string(), r.incomplete))
        .collect();
    assert_eq!(
        marked,
        [
            ("All Owners".to_owned(), true),
            ("Owner alpha".to_owned(), true),
            ("Repo alpha/app".to_owned(), false),
            ("Repo alpha/big".to_owned(), true),
            ("Owner beta".to_owned(), false),
            ("Repo beta/lib".to_owned(), false),
        ]
    );
    // The sidebar agrees with the header's per-Repo coverage.
    for repo in &view.header.repos {
        let row = sidebar_row(&view, &Scope::Repo(repo_id(&repo.repo)));
        assert_eq!(row.incomplete, !repo.complete, "{}", repo.repo);
    }
}

#[test]
fn a_narrowed_empty_scope_judges_its_own_coverage() {
    let mut harness = Harness::with_owners(&["alpha", "beta", "gamma"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .fully_read_owner("gamma")
        .owner_coverage("alpha", SourceKind::Repos, pagination_stopped())
        .pull_request(pr("gamma/app", 1))
        .build()));
    harness.start();

    harness.key(Key::Right);
    assert_eq!(harness.view.header.scope, "Owner · alpha");
    let alpha = empty_title(&harness.view);
    assert_eq!(
        alpha,
        "No Work Items found in Owner alpha, but the picture is incomplete."
    );

    harness.key(Key::Right);
    assert_eq!(harness.view.header.scope, "Owner · beta");
    assert_eq!(empty_title(&harness.view), "Nothing in Owner beta.");
}

#[test]
fn the_detail_pane_dates_kept_data_by_when_it_was_observed() {
    let mut harness = Harness::with_owners(&["alpha", "beta"]);
    harness.answer(Ok(ObservationBuilder::new()
        .observed_at(EARLIER)
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .pull_request(pr("alpha/app", 1))
        .pull_request(pr("beta/lib", 2))
        .build()));
    harness.start();
    // The refresh reads alpha but fails beta's PRs: beta/lib#2 is kept from 10:00.
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .owner_coverage(
            "beta",
            SourceKind::PullRequests,
            SourceState::Failed {
                error: "gh timed out".to_owned(),
            },
        )
        .pull_request(pr("alpha/app", 1))
        .build()));
    let effects = harness.key(Key::Char('r'));
    let survey = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .expect("r asks for a survey")
        .clone();
    harness.perform(&survey);

    let mut observed = Vec::new();
    for index in 0..harness.view.list.rows.len() {
        if index > 0 {
            harness.key(Key::Down);
        }
        let detail = harness.view.detail.content.clone().expect("selected");
        observed.push((detail.reference, detail.observed));
    }
    observed.sort();
    assert_eq!(
        observed,
        [
            ("alpha/app#1".to_owned(), "12:00 UTC, just now".to_owned()),
            ("beta/lib#2".to_owned(), "10:00 UTC, 2 h ago".to_owned()),
        ]
    );
}

#[test]
fn a_refresh_with_a_partly_read_repo_list_keeps_earlier_items_of_the_repos_it_missed() {
    let mut harness = Harness::with_owners(&["alpha"]);
    harness.answer(Ok(ObservationBuilder::new()
        .observed_at(EARLIER)
        .fully_read_owner("alpha")
        .repo("alpha/app")
        .repo("alpha/old")
        .run(run("alpha/old", "ci.yml", 7).failed())
        .issue(issue("alpha/old", 8))
        .build()));
    harness.start();
    // The refresh's Repo list stopped before alpha/old, so its Runs and Issues
    // were never asked for: they must not be dropped as if they were gone.
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .owner_coverage("alpha", SourceKind::Repos, pagination_stopped())
        .repo("alpha/app")
        .build()));
    let effects = harness.key(Key::Char('r'));
    let survey = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .expect("r asks for a survey")
        .clone();
    harness.perform(&survey);

    let rows: Vec<(String, bool)> = harness
        .view
        .list
        .rows
        .iter()
        .map(|r| (format!("{}{}", r.repo, r.reference), r.incomplete))
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|(_, incomplete)| *incomplete), "{rows:?}");
    assert!(
        harness.view.header.coverage.contains("10:00 UTC"),
        "kept data says how old it is: {}",
        harness.view.header.coverage
    );
}

#[test]
fn r_refreshes_while_the_sidebar_has_focus() {
    let mut harness = Harness::with_owners(&["alpha"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .build()));
    harness.start();
    harness.key(Key::Tab);
    assert!(harness.view.sidebar.focused);
    let effects = harness.key(Key::Char('r'));
    assert!(
        effects.iter().any(|e| matches!(e, Effect::Survey { .. })),
        "{effects:?}"
    );
    assert!(
        harness
            .view
            .footer
            .hints
            .iter()
            .any(|h| h.key == "r" && h.label == "refresh"),
        "the footer offers what works"
    );
}
