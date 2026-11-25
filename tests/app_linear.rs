//! Seam 2, Linear Workflows (#32): the App asks Linear only for Owners with a
//! Linear Workflow; their assigned Linear Issues show in the list (Linear
//! glyph by kind, identifier) and the detail (team, state, assignee,
//! priority, cycle, URL); the sidebar and header mark the Owner; Linear
//! coverage counts like GitHub's.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Event, Key,
        view_model::{LaunchStatus, Overlay},
    },
    domain::{
        Coverage, LinearPart, LinearSourceKind, NextStep, Source, SourceKind, SourceState,
        WaitingOn, WorkItemKind, facts::LinearPriority,
    },
    ordering::Scope,
    ports::LinearSurvey,
};
use support::{
    build::{ME, ObservationBuilder, linear_issue, linear_viewer, now, pr},
    harness::{Harness, config_with_linear},
};

fn linear_effects(harness: &Harness) -> usize {
    harness
        .effects
        .iter()
        .filter(|e| matches!(e, Effect::SurveyLinear { .. }))
        .count()
}

fn github_answer(owners: &[&str]) -> den::domain::Observation {
    let mut builder = ObservationBuilder::new();
    for owner in owners {
        builder = builder.fully_read_owner(owner);
    }
    builder
        .repo("acme/api")
        .pull_request(pr("acme/api", 1).author(ME).draft())
        .build()
}

/// `acme`'s Linear survey: its viewer and `issues`, every source complete.
fn linear_answer(issues: Vec<den::domain::facts::LinearIssue>) -> LinearSurvey {
    LinearSurvey {
        authentication_failed: false,
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues,
            ..LinearPart::default()
        },
        coverage: linear_coverage(SourceState::Complete),
    }
}

fn linear_coverage(issues: SourceState) -> Coverage {
    let mut coverage = Coverage::default();
    coverage.record(
        Source::Owner {
            owner: "acme".to_owned(),
            kind: SourceKind::Linear(LinearSourceKind::Viewer),
        },
        SourceState::Complete,
    );
    coverage.record(
        Source::Owner {
            owner: "acme".to_owned(),
            kind: SourceKind::Linear(LinearSourceKind::AssignedIssues),
        },
        issues,
    );
    coverage
}

fn workflow_harness() -> Harness {
    Harness::new(Ok(config_with_linear(
        &["acme", "octo"],
        &[("acme", "acme")],
    )))
    .with_linear_pairing("acme/api", "acme")
}

#[test]
fn without_a_linear_workflow_startup_and_refresh_ask_linear_nothing() {
    let mut harness = Harness::with_owners(&["acme", "octo"]);
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    assert_eq!(harness.survey_effects(), 2, "both GitHub surveys ran");
    assert_eq!(linear_effects(&harness), 0, "{:#?}", harness.effects);
    assert!(harness.linear.requests().is_empty());
    let view = &harness.view;
    assert!(view.sidebar.rows.iter().all(|r| !r.linear_workflow));
    assert!(view.header.scopes.iter().all(|s| !s.linear_workflow));
    assert!(
        view.list
            .rows
            .iter()
            .all(|r| r.kind != WorkItemKind::LinearIssue)
    );
}

#[test]
fn startup_and_refresh_survey_linear_for_the_workflow_owners_only() {
    let mut harness = workflow_harness();
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.linear.push_answer(linear_answer(Vec::new()));
    harness.linear.push_answer(linear_answer(Vec::new()));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    assert_eq!(linear_effects(&harness), 2);
    let requests = harness.linear.requests();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let owners: Vec<&str> = request.workflows.iter().map(|w| w.owner.as_str()).collect();
        assert_eq!(owners, ["acme"], "octo has no Linear Workflow");
    }
    // Linear is asked first, so its answer is in hand when GitHub's arrives.
    let first_linear = harness
        .effects
        .iter()
        .position(|e| matches!(e, Effect::SurveyLinear { .. }));
    let first_github = harness
        .effects
        .iter()
        .position(|e| matches!(e, Effect::Survey { .. }));
    assert!(first_linear < first_github, "{:#?}", harness.effects);
}

#[test]
fn assigned_linear_issues_appear_with_identifier_glyph_kind_and_implement() {
    let mut harness = workflow_harness();
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.linear.push_answer(linear_answer(vec![
        linear_issue("acme", "id-7", "TEAM-7")
            .title("Wire the thing")
            .priority(LinearPriority::Urgent)
            .cycle(12, true)
            .build(),
    ]));
    let view = harness.start().clone();

    let row = view
        .list
        .rows
        .iter()
        .find(|r| r.kind == WorkItemKind::LinearIssue)
        .unwrap_or_else(|| panic!("a Linear row: {:#?}", view.list.rows));
    assert_eq!(row.reference, "TEAM-7");
    assert_eq!(row.repo, "acme", "a Linear Issue's place is its Owner");
    assert_eq!(row.title, "Wire the thing");
    assert_eq!(row.next_step, Some(NextStep::Implement));
    assert_eq!(row.waiting_on, WaitingOn::Me);
    assert!(!row.incomplete);

    // Select it and read the detail (story 17).
    let index = view
        .list
        .rows
        .iter()
        .position(|r| r.kind == WorkItemKind::LinearIssue)
        .unwrap();
    for _ in 0..index {
        harness.key(Key::Down);
    }
    let detail = harness.view.detail.content.clone().expect("selected");
    assert!(detail.reference.contains("TEAM-7"), "{detail:?}");
    assert!(detail.reference.contains("Linear"), "{detail:?}");
    assert_eq!(detail.url, "https://linear.app/acme/issue/team-7");
    let facts = detail.facts.expect("Linear facts").lines.join(" | ");
    for expected in [
        "team TEAM",
        "In Progress",
        "assignee Me Myself",
        "priority urgent",
        "cycle 12, active",
    ] {
        assert!(facts.contains(expected), "{expected:?} in {facts}");
    }
    assert_eq!(detail.waiting_on, "you");
}

#[test]
fn the_sidebar_marks_only_the_paired_repo_and_owner_scope_deactivates_it() {
    let mut harness = workflow_harness();
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.linear.push_answer(linear_answer(Vec::new()));
    let view = harness.start().clone();

    let owner_row = |owner: &str| {
        view.sidebar
            .rows
            .iter()
            .find(|r| r.scope == Scope::Owner(owner.to_owned()))
            .expect("owner row")
    };
    assert!(!owner_row("acme").linear_workflow);
    assert!(view.sidebar.rows.iter().any(|row| row.scope
        == Scope::Repo(den::domain::RepoId::new("acme", "api"))
        && row.linear_workflow));
    assert!(!owner_row("octo").linear_workflow);
    let segment = |label: &str| {
        view.header
            .scopes
            .iter()
            .find(|s| s.label == label)
            .expect("segment")
    };
    // Activation marks the selected Repo, never its whole Owner.
    assert!(!segment("acme").linear_workflow);
    assert!(segment("api").linear_workflow);
    assert!(!segment("octo").linear_workflow);

    // Owner Scope has no selected Repo, so it removes Linear eligibility.
    harness.key(Key::Tab);
    let acme = harness
        .view
        .sidebar
        .rows
        .iter()
        .position(|r| r.scope == Scope::Owner("acme".to_owned()))
        .unwrap();
    let current = harness.view.sidebar.cursor.unwrap_or(0);
    for _ in 0..current.abs_diff(acme) {
        harness.key(if current > acme { Key::Up } else { Key::Down });
    }
    harness.key(Key::Enter);
    assert!(
        !harness.view.header.scope.contains("Linear Workflow"),
        "{}",
        harness.view.header.scope
    );
}

/// Story 88 for the tracer: a Linear source that failed is incomplete
/// coverage like a GitHub one, named in the header, and blocks "Nothing
/// needs you".
/// Repo Scopes hold the Linear Issues with a member PR in them (Landing
/// Groups, #35), so a Linear source den couldn't read may hide work in any
/// Repo of its Owner: a failed Linear source leaves the Owner, All Owners and
/// every Repo of the Owner incomplete.
#[test]
fn a_failed_linear_source_leaves_the_owners_repos_incomplete() {
    let mut harness = workflow_harness();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .repo("acme/api")
        .build()));
    harness.linear.push_answer(LinearSurvey {
        authentication_failed: false,
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            ..LinearPart::default()
        },
        coverage: linear_coverage(SourceState::Failed {
            error: "Linear did not answer within 20s".to_owned(),
        }),
    });
    let view = harness.start().clone();
    let row = |scope: Scope| {
        view.sidebar
            .rows
            .iter()
            .find(|r| r.scope == scope)
            .unwrap_or_else(|| panic!("{scope:?} row: {:#?}", view.sidebar.rows))
            .clone()
    };
    assert!(row(Scope::Owner("acme".to_owned())).incomplete);
    let api = Scope::Repo(den::domain::RepoId::new("acme", "api"));
    // Inverted with #35: before Landing Groups no Linear Issue could be in a
    // Repo Scope, so this asserted complete.
    assert!(
        row(api).incomplete,
        "a Linear Issue with a member PR in acme/api may be unread"
    );
    let api_coverage = view
        .header
        .repos
        .iter()
        .find(|r| r.repo == "acme/api")
        .expect("acme/api coverage");
    assert!(!api_coverage.complete, "{api_coverage:?}");
}

#[test]
fn a_failed_linear_source_shows_in_coverage_and_blocks_nothing_needs_you() {
    let mut harness = workflow_harness();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build()));
    harness.linear.push_answer(LinearSurvey {
        authentication_failed: false,
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues: Vec::new(),
            ..LinearPart::default()
        },
        coverage: linear_coverage(SourceState::Failed {
            error: "Linear did not answer within 20s".to_owned(),
        }),
    });
    let view = harness.start().clone();
    let empty = view.list.empty.expect("no rows");
    assert_ne!(empty.title, "Nothing needs you.");
    assert!(!view.header.coverage_complete);
    assert!(
        view.header
            .coverage
            .contains("assigned Linear Issues of acme"),
        "{}",
        view.header.coverage
    );
    let acme_row = view
        .sidebar
        .rows
        .iter()
        .find(|r| r.scope == Scope::Owner("acme".to_owned()))
        .unwrap();
    assert!(acme_row.incomplete);
    let octo_row = view
        .sidebar
        .rows
        .iter()
        .find(|r| r.scope == Scope::Owner("octo".to_owned()))
        .unwrap();
    assert!(!octo_row.incomplete, "octo has no Linear Workflow");
}

/// A complete paired-project survey gives a Repo-specific empty state
/// when there is nothing.
#[test]
fn a_complete_linear_survey_with_nothing_assigned_shows_the_empty_repo() {
    let mut harness = workflow_harness();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build()));
    harness.linear.push_answer(linear_answer(Vec::new()));
    let view = harness.start().clone();
    // Eligibility is Repo-specific, so the empty state names that Repo.
    assert_eq!(
        view.list.empty.expect("empty").title,
        "Nothing in Repo acme/api."
    );
    assert!(view.header.coverage_complete);
}

/// Linear coverage comes from the Linear source only: if the Linear survey
/// isn't answered, its sources are not yet fetched, never complete.
#[test]
fn a_linear_survey_that_never_answered_is_not_yet_fetched() {
    let mut harness = workflow_harness();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build()));
    let effects = harness.send(Event::Started { now: now() });
    // Perform everything but the Linear survey.
    for effect in effects
        .iter()
        .filter(|e| !matches!(e, Effect::SurveyLinear { .. }))
    {
        harness.perform(effect);
    }
    let view = harness.view.clone();
    assert_ne!(
        view.list.empty.expect("empty").title,
        "Nothing needs you.",
        "{}",
        view.header.coverage
    );
    assert!(
        view.header.coverage.contains("not fetched yet"),
        "{}",
        view.header.coverage
    );
}

/// The launch sheet for a Linear Issue whose Owner's Linear Workflow offers
/// no Repos for new work refuses, saying why, and inspects nothing. (Before
/// #38 every Linear Issue refused here; `app_linear_writes.rs` launches one
/// into a Repo the workflow offers.)
#[test]
fn c_on_a_linear_issue_refuses_without_touching_the_workspace() {
    let mut harness = workflow_harness();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build()));
    harness.linear.push_answer(linear_answer(vec![
        linear_issue("acme", "id-7", "TEAM-7").build(),
    ]));
    harness.start();
    let effects = harness.key(Key::Char('c'));
    assert!(
        effects
            .iter()
            .all(|e| !matches!(e, Effect::InspectLaunch { .. } | Effect::Launch { .. })),
        "{effects:#?}"
    );
    let Some(Overlay::LaunchSheet(sheet)) = &harness.view.overlay else {
        panic!("a sheet: {:?}", harness.view.overlay);
    };
    assert_eq!(sheet.status, LaunchStatus::Refused);
    assert!(
        sheet
            .messages
            .iter()
            .any(|m| m.text.contains("no facts about the Repo acme/api")),
        "{:?}",
        sheet.messages
    );
}

/// The Linear part follows the latest Linear answer: a refresh that no longer
/// lists an issue drops it.
#[test]
fn a_refresh_replaces_the_linear_part() {
    let mut harness = workflow_harness();
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.linear.push_answer(linear_answer(vec![
        linear_issue("acme", "id-7", "TEAM-7").build(),
    ]));
    harness.linear.push_answer(linear_answer(Vec::new()));
    harness.start();
    let linear_rows = |h: &Harness| {
        h.view
            .list
            .rows
            .iter()
            .filter(|r| r.kind == WorkItemKind::LinearIssue)
            .count()
    };
    assert_eq!(linear_rows(&harness), 1);
    harness.settle(Event::Key(Key::Char('r')));
    assert_eq!(linear_rows(&harness), 0);
}

/// Performs `effects` in order, refusing each Linear survey as a full
/// worker queue does.
fn perform_refusing_linear(harness: &mut Harness, effects: Vec<Effect>) {
    for effect in effects {
        if matches!(effect, Effect::SurveyLinear { .. }) {
            harness.send(Event::EffectRejected {
                effect: Box::new(effect),
                reason: "den is still busy with earlier requests; try again shortly".to_owned(),
            });
        } else {
            harness.perform(&effect);
        }
    }
}

/// A refused Linear survey asked Linear nothing: its sources read as
/// failed for that survey, never as the previous answer, complete.
#[test]
fn a_refused_linear_survey_reads_as_failed_not_as_the_last_answer() {
    let mut harness = workflow_harness();
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.linear.push_answer(linear_answer(Vec::new()));
    harness.start();
    assert!(
        harness.view.header.coverage_complete,
        "the first survey read everything"
    );

    let effects = harness.key(Key::Char('r'));
    perform_refusing_linear(&mut harness, effects);

    let header = &harness.view.header;
    assert!(!header.coverage_complete, "{}", header.coverage);
    assert!(
        header.coverage.contains("assigned Linear Issues of acme")
            && header.coverage.contains("could not ask Linear"),
        "{}",
        header.coverage
    );
    assert_eq!(harness.linear.requests().len(), 1, "Linear was asked once");
}

/// The cache's Linear facts are fitted to the Linear Workflows configured
/// now, per Owner and workspace: an Owner whose workflow is gone, or names
/// another workspace, loses its cached Linear Issues.
#[test]
fn the_cached_linear_part_is_fitted_to_each_owners_workflow() {
    let cached = ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .fully_read_owner("ursa")
        .linear_viewer("acme", "acme")
        .linear_viewer("octo", "octo")
        .linear_viewer("ursa", "ursa")
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .linear_issue(linear_issue("octo", "id-2", "TEAM-2").workspace("octo"))
        .linear_issue(linear_issue("ursa", "id-3", "TEAM-3").workspace("ursa"))
        .build();
    // acme keeps its workflow; octo's is gone; ursa's names a new workspace.
    let config = config_with_linear(
        &["acme", "octo", "ursa"],
        &[("acme", "acme"), ("ursa", "ursa-two")],
    );
    let mut harness = Harness::with_cache(
        Ok(config),
        support::fakes::FakeCache::holding(support::harness::paired_cache(
            den::cache::CachedObservation::fresh(cached),
            "acme/api",
        )),
    )
    .with_linear_pairing("acme/api", "acme");
    let effects = harness.send(Event::Started { now: now() });
    let load = effects
        .iter()
        .find(|e| matches!(e, Effect::LoadCache))
        .expect("the cache is read");
    harness.perform(load);

    let shown: Vec<String> = harness
        .view
        .list
        .rows
        .iter()
        .filter(|r| r.kind == WorkItemKind::LinearIssue)
        .map(|r| r.reference.clone())
        .collect();
    assert_eq!(
        shown,
        vec!["TEAM-1".to_owned()],
        "{:#?}",
        harness.view.list.rows
    );
    let coverage = &harness.view.header.coverage;
    // An unpaired Owner does not contribute Linear coverage requirements.
    assert!(harness.view.header.coverage_complete, "{coverage}");
}

/// A Linear Workflow added since a complete GitHub-only cache was saved:
/// until Linear answers, its sources are not yet fetched, so the cached
/// Observation never says "Nothing needs you".
#[test]
fn a_new_linear_workflow_is_not_yet_fetched_in_a_github_only_cache() {
    let cached = ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build();
    let mut harness = Harness::with_cache(
        Ok(config_with_linear(&["acme", "octo"], &[("acme", "acme")])),
        support::fakes::FakeCache::holding(support::harness::paired_cache(
            den::cache::CachedObservation::fresh(cached),
            "acme/api",
        )),
    )
    .with_linear_pairing("acme/api", "acme");
    let effects = harness.send(Event::Started { now: now() });
    let load = effects
        .iter()
        .find(|e| matches!(e, Effect::LoadCache))
        .expect("the cache is read");
    harness.perform(load);

    let view = &harness.view;
    assert!(!view.header.coverage_complete, "{}", view.header.coverage);
    assert!(
        view.header.coverage.contains("not fetched yet"),
        "{}",
        view.header.coverage
    );
    assert_ne!(
        view.list.empty.as_ref().expect("empty").title,
        "Nothing needs you."
    );
}

fn not_logged_in() -> den::ports::SurveyFailure {
    den::ports::SurveyFailure::NotLoggedIn {
        detail: "gh auth status: not logged in".to_owned(),
    }
}

/// On a first run (nothing cached), Linear's answer shows even when the
/// GitHub survey fails, whichever answers first; it is never saved (the
/// cache is per GitHub viewer, and den doesn't know who that is).
#[test]
fn linear_issues_show_when_the_first_github_survey_fails() {
    for github_first in [false, true] {
        let mut harness = workflow_harness();
        harness.answer(Err(not_logged_in()));
        harness.linear.push_answer(linear_answer(vec![
            linear_issue("acme", "id-7", "TEAM-7").build(),
        ]));
        let mut effects = harness.send(Event::Started { now: now() });
        if github_first {
            effects.sort_by_key(|e| matches!(e, Effect::SurveyLinear { .. }));
        }
        for effect in effects {
            harness.perform(&effect);
        }

        let view = &harness.view;
        let linear_rows: Vec<&str> = view
            .list
            .rows
            .iter()
            .filter(|r| r.kind == WorkItemKind::LinearIssue)
            .map(|r| r.reference.as_str())
            .collect();
        assert_eq!(linear_rows, vec!["TEAM-7"], "github first: {github_first}");
        assert!(!view.header.coverage_complete);
        assert!(
            view.header.status.contains("not logged in"),
            "{}",
            view.header.status
        );
        assert!(harness.cache.stored().is_empty(), "nothing saved");
    }
}

/// One refresh writes the cache once: Linear's answer, when GitHub's is
/// still to come, is saved with it. If that GitHub survey fails, the
/// failure saves the Linear answer instead.
#[test]
fn a_refresh_saves_the_cache_once() {
    let mut harness = workflow_harness();
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.answer(Ok(github_answer(&["acme", "octo"])));
    harness.answer(Err(not_logged_in()));
    harness.linear.push_answer(linear_answer(Vec::new()));
    harness.linear.push_answer(linear_answer(Vec::new()));
    harness.linear.push_answer(linear_answer(vec![
        linear_issue("acme", "id-7", "TEAM-7").build(),
    ]));
    harness.start();
    assert_eq!(harness.cache.stored().len(), 1);

    harness.settle(Event::Key(Key::Char('r')));
    assert_eq!(harness.cache.stored().len(), 2, "one write for the refresh");

    harness.settle(Event::Key(Key::Char('r')));
    let stored = harness.cache.stored();
    assert_eq!(
        stored.len(),
        3,
        "the failed refresh saved Linear's answer once"
    );
    assert_eq!(stored[2].observation.linear.issues.len(), 1);
}
