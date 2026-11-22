//! Seam 2: the App driven by events, with fake GitHub, Workspace and Launcher
//! ports. Asserts on the ViewModel and the Effects only.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Event, Key, LogLevel, SurveyId},
    config::{self, ConfigError},
    domain::{
        Coverage, Incompleteness, LinearPart, LinearSourceKind, NextStep, Source, SourceKind,
        SourceState, UpstreamTargets, WaitingOn,
    },
    ports::{LinearSurvey, SurveyFailure, SurveyRequest, SurveyTarget},
};
use std::path::{Path, PathBuf};
use support::{
    build::{ME, ObservationBuilder, now, pr},
    harness::{Harness, config_with_linear},
};

#[test]
fn startup_surveys_the_configured_owners_and_lists_their_open_prs() {
    let mut harness = Harness::with_owners(&["alpha", "beta"]);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("alpha")
        .owner("beta")
        .pull_request(pr("alpha/app", 7).draft().author(ME).title("Half done"))
        .pull_request(pr("beta/app", 7).author("alice").title("Ready"))
        .build()));
    let view = harness.start().clone();

    assert_eq!(
        harness.github.requests(),
        [SurveyRequest {
            target: SurveyTarget::Owners(vec!["alpha".to_owned(), "beta".to_owned()]),
            upstreams: UpstreamTargets::default(),
            screenshot_repos: Vec::new(),
        }]
    );
    assert_eq!(harness.survey_effects(), 1);
    assert_eq!(
        harness.rows(),
        [
            ("alpha/app".into(), "#7".into(), "Finish draft".into()),
            ("beta/app".into(), "#7".into(), "no step yet".into()),
        ]
    );
    let draft = &view.list.rows[0];
    assert_eq!(draft.next_step, Some(NextStep::FinishDraft));
    assert_eq!(draft.waiting_on, WaitingOn::Me);
    assert_eq!(draft.title, "Half done");
    assert_eq!(view.list.rows[1].next_step, None);
    assert_eq!(view.list.selected, Some(0));

    let detail = view.detail.content.expect("the first row is selected");
    assert_eq!(detail.next_step, "Finish draft");
    assert_eq!(detail.waiting_on, "you");
    assert!(detail.why.contains("draft"), "{}", detail.why);
    assert_eq!(detail.url, "https://github.com/alpha/app/pull/7");
    assert!(view.header.scope.contains("alpha") && view.header.scope.contains("beta"));
    assert!(
        view.header.status.contains("12:00 UTC"),
        "{}",
        view.header.status
    );
}

#[test]
fn a_missing_config_is_reported_with_its_path_and_nothing_is_surveyed() {
    let path = PathBuf::from("/home/someone/.config/den/config.toml");
    let mut harness = Harness::new(Err(ConfigError::Missing { path: path.clone() }));
    let view = harness.start().clone();

    assert_eq!(harness.survey_effects(), 0);
    assert!(harness.github.requests().is_empty());
    let empty = view.list.empty.expect("an empty state");
    assert!(
        empty.detail.contains(&path.display().to_string()),
        "{empty:?}"
    );
    assert!(
        empty.detail.contains("owners"),
        "says what to write: {empty:?}"
    );
    assert!(harness.effects.iter().any(
        |e| matches!(e, Effect::Log(r) if r.level == LogLevel::Error && r.message.contains("config"))
    ));
}

#[test]
fn a_malformed_config_is_reported_with_file_and_line() {
    let path = Path::new("/tmp/den/config.toml");
    let error = config::parse(path, "# den\nowners = [\"ok\", 42]\n").expect_err("malformed");
    let mut harness = Harness::new(Err(error));
    let view = harness.start().clone();

    assert_eq!(harness.survey_effects(), 0);
    let detail = view.list.empty.expect("an empty state").detail;
    assert!(detail.contains("/tmp/den/config.toml"), "{detail}");
    assert!(detail.contains("line 2"), "{detail}");
}

#[test]
fn gh_missing_or_logged_out_says_what_to_fix() {
    let cases = [
        (
            SurveyFailure::GhNotInstalled,
            "gh is not installed",
            ["https://cli.github.com", "gh auth login"],
        ),
        (
            SurveyFailure::NotLoggedIn {
                detail: "To get started with GitHub CLI, please run:  gh auth login".to_owned(),
            },
            "gh is not logged in",
            ["gh auth login", "not logged in"],
        ),
    ];
    for (failure, title, mentions) in cases {
        let mut harness = Harness::with_owners(&["alpha"]);
        harness.answer(Err(failure.clone()));
        let view = harness.start().clone();
        let empty = view.list.empty.expect("an empty state");
        assert_eq!(empty.title, title);
        for mention in mentions {
            assert!(empty.detail.contains(mention), "{failure:?}: {empty:?}");
        }
        assert_eq!(view.header.status, title);
        assert!(harness.effects.iter().any(
            |e| matches!(e, Effect::Log(r) if r.level == LogLevel::Error && r.message.contains("failed"))
        ));
    }
}

#[test]
fn errors_alongside_data_show_as_incomplete_not_as_success() {
    let partial = SourceState::Partial(Incompleteness::ErrorsAlongsideData {
        errors: vec!["Resource not accessible by integration".to_owned()],
    });
    let mut harness = Harness::with_owners(&["alpha"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .owner_coverage("alpha", SourceKind::PullRequests, partial)
        .pull_request(pr("alpha/app", 1))
        .build()));
    let view = harness.start().clone();

    assert!(!view.header.coverage_complete);
    assert!(
        view.header.coverage.contains("Pull Requests of alpha")
            && view.header.coverage.contains("Resource not accessible"),
        "{}",
        view.header.coverage
    );
    assert!(view.list.rows[0].incomplete);
    let detail = view.detail.content.expect("selected");
    assert!(
        detail.coverage.iter().any(|c| c.contains("partial")),
        "{:?}",
        detail.coverage
    );
}

/// Milestone-2 groundwork (#31): an Owner without a Linear Workflow records
/// no Linear source, so the missing Linear part never stops "Nothing needs
/// you"; a Linear source once recorded counts like any GitHub one.
///
/// Changed in #32: the Linear sources used to arrive on the GitHub survey's
/// Observation, because no Linear source existed. They now come from the
/// Linear source of an Owner with a Linear Workflow (the App drops any Linear
/// coverage a GitHub answer carries, as GitHub can't report on Linear), so
/// the incomplete cases configure one and script the Linear answer. Not yet
/// fetched is a Linear survey that recorded nothing for the source, which
/// the App must still count as not fetched.
#[test]
fn linear_sources_count_toward_nothing_needs_you_only_when_recorded() {
    let mut github_only = Harness::with_owners(&["alpha"]);
    github_only.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .build()));
    let view = github_only.start().clone();
    assert_eq!(view.list.empty.expect("empty").title, "Nothing needs you.");

    for state in [
        SourceState::NotYetFetched,
        SourceState::Failed {
            error: "Linear did not answer".to_owned(),
        },
    ] {
        let mut harness = Harness::new(Ok(config_with_linear(&["alpha"], &[("alpha", "acme")])));
        harness.answer(Ok(ObservationBuilder::new()
            .fully_read_owner("alpha")
            .build()));
        let mut coverage = Coverage::default();
        coverage.record(
            Source::Owner {
                owner: "alpha".to_owned(),
                kind: SourceKind::Linear(LinearSourceKind::Viewer),
            },
            SourceState::Complete,
        );
        if state != SourceState::NotYetFetched {
            coverage.record(
                Source::Owner {
                    owner: "alpha".to_owned(),
                    kind: SourceKind::Linear(LinearSourceKind::AssignedIssues),
                },
                state.clone(),
            );
        }
        harness.linear.push_answer(LinearSurvey {
            part: LinearPart::default(),
            coverage,
        });
        let view = harness.start().clone();
        let empty = view.list.empty.expect("empty");
        assert_ne!(empty.title, "Nothing needs you.", "{state:?}");
        assert!(
            empty.detail.contains("assigned Linear Issues"),
            "says which Linear source: {empty:?}"
        );
        assert!(!view.header.coverage_complete, "{state:?}");
    }
}

#[test]
fn nothing_needs_you_only_when_the_survey_was_complete() {
    let mut complete = Harness::with_owners(&["alpha"]);
    complete.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .build()));
    let view = complete.start().clone();
    assert_eq!(view.list.empty.expect("empty").title, "Nothing needs you.");
    assert!(view.header.coverage_complete);

    for (name, observation) in [
        (
            "kinds not fetched yet",
            ObservationBuilder::new().owner("alpha").build(),
        ),
        (
            "rate limited",
            ObservationBuilder::new()
                .fully_read_owner("alpha")
                .owner_coverage(
                    "alpha",
                    SourceKind::PullRequests,
                    SourceState::Partial(Incompleteness::RateLimited {
                        resets_at: Some(now()),
                    }),
                )
                .build(),
        ),
        (
            "an Owner failed",
            ObservationBuilder::new()
                .fully_read_owner("alpha")
                .owner_coverage(
                    "alpha",
                    SourceKind::Repos,
                    SourceState::Failed {
                        error: "GitHub has no user or organization called alpha".to_owned(),
                    },
                )
                .build(),
        ),
    ] {
        let mut harness = Harness::with_owners(&["alpha"]);
        harness.answer(Ok(observation));
        let view = harness.start().clone();
        let empty = view.list.empty.expect("empty");
        assert_ne!(empty.title, "Nothing needs you.", "{name}");
        assert!(!empty.detail.is_empty(), "{name}: says what is missing");
        assert!(!view.header.coverage_complete, "{name}");
    }
}

#[test]
fn rows_waiting_on_me_come_first_and_someone_elses_are_dimmed() {
    let mut harness = Harness::with_owners(&["o"]);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("o")
        .pull_request(
            pr("o/r", 1)
                .draft()
                .author("alice")
                .updated("2026-11-19T11:00:00Z"),
        )
        .pull_request(pr("o/r", 2).updated("2026-11-19T10:00:00Z"))
        .pull_request(
            pr("o/r", 3)
                .draft()
                .author(ME)
                .updated("2026-11-01T00:00:00Z"),
        )
        .build()));
    let view = harness.start().clone();
    let order: Vec<(&str, WaitingOn, bool)> = view
        .list
        .rows
        .iter()
        .map(|r| (r.reference.as_str(), r.waiting_on, r.dimmed))
        .collect();
    assert_eq!(
        order,
        [
            ("#3", WaitingOn::Me, false),
            ("#2", WaitingOn::Nobody, false),
            ("#1", WaitingOn::SomeoneElse, true),
        ]
    );
}

#[test]
fn arrows_move_the_selection_within_the_list_and_q_quits() {
    let mut harness = Harness::with_owners(&["o"]);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("o")
        .pull_request(pr("o/r", 1).updated("2026-11-19T11:00:00Z"))
        .pull_request(pr("o/r", 2).updated("2026-11-19T10:00:00Z"))
        .build()));
    harness.start();
    assert_eq!(harness.view.list.selected, Some(0));
    harness.key(Key::Up);
    assert_eq!(harness.view.list.selected, Some(0), "stays at the top");
    harness.key(Key::Down);
    assert_eq!(harness.view.list.selected, Some(1));
    harness.key(Key::Down);
    assert_eq!(harness.view.list.selected, Some(1), "stays at the bottom");
    assert_eq!(
        harness
            .view
            .detail
            .content
            .as_ref()
            .map(|d| d.reference.as_str()),
        Some("o/r#2")
    );
    assert_eq!(harness.key(Key::Char('q')), [Effect::Quit]);
    assert_eq!(harness.key(Key::Interrupt), [Effect::Quit]);
}

#[test]
fn a_late_answer_to_an_older_survey_is_ignored() {
    let mut harness = Harness::with_owners(&["o"]);
    let effects = harness.send(Event::Started { now: now() });
    let Some(Effect::Survey { id, .. }) = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .cloned()
    else {
        panic!("a survey effect: {effects:?}");
    };
    let before = harness.view.clone();
    let stale = SurveyId(id.0 + 1);
    let effects = harness.send(Event::SurveyFinished {
        id: stale,
        outcome: Ok(Box::new(
            ObservationBuilder::new()
                .owner("o")
                .pull_request(pr("o/r", 1))
                .build(),
        )),
        now: now(),
    });
    assert_eq!(harness.view, before);
    assert!(matches!(effects.as_slice(), [Effect::Log(r)] if r.level == LogLevel::Warn));
    assert_eq!(before.header.status, "Surveying…");
}
