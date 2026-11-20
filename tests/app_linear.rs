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
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues,
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
fn the_sidebar_and_header_mark_an_owner_with_a_linear_workflow() {
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
    assert!(owner_row("acme").linear_workflow);
    assert!(!owner_row("octo").linear_workflow);
    let segment = |label: &str| {
        view.header
            .scopes
            .iter()
            .find(|s| s.label == label)
            .expect("segment")
    };
    assert!(segment("acme").linear_workflow);
    assert!(!segment("octo").linear_workflow);

    // In acme's Scope the sentence says it in words, too.
    harness.key(Key::Tab);
    let acme = harness
        .view
        .sidebar
        .rows
        .iter()
        .position(|r| r.scope == Scope::Owner("acme".to_owned()))
        .unwrap();
    for _ in 0..acme {
        harness.key(Key::Down);
    }
    harness.key(Key::Enter);
    assert!(
        harness.view.header.scope.contains("Linear Workflow"),
        "{}",
        harness.view.header.scope
    );
}

/// Story 88 for the tracer: a Linear source that failed is incomplete
/// coverage like a GitHub one, named in the header, and blocks "Nothing
/// needs you".
#[test]
fn a_failed_linear_source_shows_in_coverage_and_blocks_nothing_needs_you() {
    let mut harness = workflow_harness();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build()));
    harness.linear.push_answer(LinearSurvey {
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues: Vec::new(),
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

/// A complete survey with a Linear Workflow still says "Nothing needs you"
/// when there is nothing.
#[test]
fn a_complete_linear_survey_with_nothing_assigned_is_nothing_needs_you() {
    let mut harness = workflow_harness();
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build()));
    harness.linear.push_answer(linear_answer(Vec::new()));
    let view = harness.start().clone();
    assert_eq!(view.list.empty.expect("empty").title, "Nothing needs you.");
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

/// The launch sheet for a Linear Issue refuses, saying why, and inspects
/// nothing (Start and Implement sheets are #38).
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
            .any(|m| m.text.contains("Linear Issue")),
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
