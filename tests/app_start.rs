//! Seam 2, Start (#33): with a Linear Workflow naming start teams, the
//! config's start settings reach the decision engine, the start teams'
//! unassigned issues show as Start Waiting On nobody, and the start-team and
//! team-states sources count toward coverage on their own: not yet fetched
//! until Linear says, and a team-states failure naming a config value shows
//! in the header and blocks "Nothing needs you".

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Event},
    cache::CachedObservation,
    config::Config,
    domain::{
        Coverage, LinearPart, LinearSourceKind, NextStep, Source, SourceKind, SourceState,
        WaitingOn, WorkItemKind, facts::LinearIssue,
    },
    ports::LinearSurvey,
};
use support::{
    build::{ObservationBuilder, linear_issue, linear_viewer, now},
    fakes::FakeCache,
    harness::{Harness, config_with_linear},
};

const ALL_FOUR: [LinearSourceKind; 4] = [
    LinearSourceKind::Viewer,
    LinearSourceKind::AssignedIssues,
    LinearSourceKind::TeamStates,
    LinearSourceKind::StartTeamIssues,
];

/// `acme` (start team `TEAM`) and `octo` (no Linear Workflow).
fn start_config() -> Config {
    let mut config = config_with_linear(&["acme", "octo"], &[("acme", "acme")]);
    config.linear[0].start_teams = vec!["TEAM".to_owned()];
    config
}

fn harness() -> Harness {
    let harness = Harness::new(Ok(start_config())).with_linear_pairing("acme/api", "acme");
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build()));
    harness
}

fn answer(issues: Vec<LinearIssue>, states: &[(LinearSourceKind, SourceState)]) -> LinearSurvey {
    let mut coverage = Coverage::default();
    for (kind, state) in states {
        coverage.record(
            Source::Owner {
                owner: "acme".to_owned(),
                kind: SourceKind::Linear(*kind),
            },
            state.clone(),
        );
    }
    LinearSurvey {
        authentication_failed: false,
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues,
            ..LinearPart::default()
        },
        coverage,
    }
}

fn complete() -> Vec<(LinearSourceKind, SourceState)> {
    ALL_FOUR
        .iter()
        .map(|k| (*k, SourceState::Complete))
        .collect()
}

#[test]
fn a_start_teams_unassigned_issue_shows_start_waiting_on_nobody() {
    let mut harness = harness();
    harness.linear.push_answer(answer(
        vec![
            linear_issue("acme", "id-8", "TEAM-8")
                .up_for_grabs()
                .title("Pick me")
                .build(),
            // Not a start team: not shown.
            linear_issue("acme", "id-9", "OPS-9").up_for_grabs().build(),
        ],
        &complete(),
    ));
    let view = harness.start().clone();

    let linear_rows: Vec<_> = view
        .list
        .rows
        .iter()
        .filter(|r| r.kind == WorkItemKind::LinearIssue)
        .collect();
    assert_eq!(linear_rows.len(), 1, "{linear_rows:#?}");
    let row = linear_rows[0];
    assert_eq!(row.reference, "TEAM-8");
    assert_eq!(row.next_step, Some(NextStep::Start));
    assert_eq!(row.step_label, "Start");
    assert_eq!(row.waiting_on, WaitingOn::Nobody);
    assert!(!row.incomplete);
    assert!(view.header.coverage_complete, "{}", view.header.coverage);

    // The request carried the config's start settings to the Linear source.
    let request = &harness.linear.requests()[0];
    assert_eq!(request.workflows[0].start_teams, ["TEAM"]);
}

/// Before Linear says anything about them, the workflow's team-states and
/// start-team sources are not yet fetched, never complete.
#[test]
fn unanswered_start_sources_are_not_yet_fetched() {
    let mut harness = harness();
    harness.linear.push_answer(answer(
        Vec::new(),
        &[
            (LinearSourceKind::Viewer, SourceState::Complete),
            (LinearSourceKind::AssignedIssues, SourceState::Complete),
        ],
    ));
    let view = harness.start().clone();
    assert!(!view.header.coverage_complete);
    assert!(view.header.coverage.contains("not fetched yet"));
    for label in ["Linear team states", "start-team Linear Issues"] {
        assert!(
            view.header.coverage.contains(label),
            "{label:?} in {}",
            view.header.coverage
        );
    }
    assert_ne!(
        view.list.empty.expect("no rows").title,
        "Nothing needs you."
    );
}

/// Cached Start coverage has no query scope. A broader config must wait for
/// its own survey before an empty cache can say that nothing needs attention.
#[test]
fn cached_start_coverage_is_unconfirmed_until_the_current_query_answers() {
    let mut cached = ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .linear_viewer("acme", "acme");
    for kind in ALL_FOUR {
        cached = cached.owner_coverage("acme", SourceKind::Linear(kind), SourceState::Complete);
    }
    let cached = CachedObservation::fresh(cached.build());
    for expansion in ["team", "backlog", "cycle", "unchanged"] {
        let mut config = start_config();
        config.linear[0].active_cycle_only = true;
        match expansion {
            "team" => config.linear[0].start_teams.push("OPS".to_owned()),
            "backlog" => config.linear[0].include_backlog = true,
            "cycle" => config.linear[0].active_cycle_only = false,
            // Even unchanged configs cannot be proven compatible with this cache.
            "unchanged" => {}
            _ => unreachable!(),
        }
        let mut harness = Harness::with_cache(Ok(config), FakeCache::holding(cached.clone()))
            .with_linear_pairing("acme/api", "acme");
        harness.answer(Ok(ObservationBuilder::new()
            .fully_read_owner("acme")
            .fully_read_owner("octo")
            .build()));
        harness.linear.push_answer(answer(Vec::new(), &complete()));
        let effects = harness.send(Event::Started { now: now() });
        let load = effects
            .iter()
            .find(|e| matches!(e, Effect::LoadCache))
            .unwrap();
        harness.perform(load);
        assert!(!harness.view.header.coverage_complete, "{expansion}");
        for label in ["start-team Linear Issues", "Linear team states"] {
            assert!(
                harness.view.header.coverage.contains(label),
                "{expansion}: {}",
                harness.view.header.coverage
            );
        }
        assert!(harness.view.header.coverage.contains("not fetched yet"));
        assert_ne!(
            harness.view.list.empty.as_ref().unwrap().title,
            "Nothing needs you."
        );
        let github = effects
            .iter()
            .find(|e| matches!(e, Effect::Survey { .. }))
            .unwrap();
        harness.perform(github);
        assert!(
            !harness.view.header.coverage_complete,
            "GitHub cannot confirm Linear: {expansion}"
        );
        let linear = effects
            .iter()
            .find(|e| matches!(e, Effect::SurveyLinear { .. }))
            .unwrap();
        harness.perform(linear);
        assert!(
            harness.view.header.coverage_complete,
            "the current query answered: {expansion}: {}",
            harness.view.header.coverage
        );
    }
}

/// A config value Linear doesn't have is a failed source the user sees,
/// naming the value, not a quietly disabled rule.
#[test]
fn a_team_states_failure_names_the_value_and_blocks_nothing_needs_you() {
    let mut harness = harness();
    let mut states = complete();
    states[2] = (
        LinearSourceKind::TeamStates,
        SourceState::Failed {
            error: "[linear.acme] names what Linear doesn't have: start_teams names \"NOPE\", but the Linear workspace acme has no team with that key".to_owned(),
        },
    );
    harness.linear.push_answer(answer(Vec::new(), &states));
    let view = harness.start().clone();
    assert!(!view.header.coverage_complete);
    assert!(
        view.header.coverage.contains("Linear team states of acme")
            && view.header.coverage.contains("\"NOPE\""),
        "{}",
        view.header.coverage
    );
    assert_ne!(
        view.list.empty.expect("no rows").title,
        "Nothing needs you."
    );
}
