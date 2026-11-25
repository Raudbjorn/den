//! Seam 2, Linear data in the cache (#34): the Linear part is cached and merged
//! per Linear source like the GitHub part (a source read completely replaces
//! its data, any other keeps the earlier data with its own age), dropped when
//! the Linear viewer behind it changes, and merged independently of GitHub.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Event, Key, ViewModel},
    cache::{CachedObservation, Carried, LinearReads, merge_linear},
    domain::{
        Coverage, Incompleteness, LinearPart, LinearSourceKind, LinearTeamStates, LinearViewer,
        NextStep, Observation, Source, SourceKind, SourceState, WaitingOn, WorkItemKind,
        facts::{LinearIssue, LinearState, LinearStateKind, LinearTeam},
    },
    ports::{LinearSurvey, SurveyFailure},
};
use support::{
    build::{ObservationBuilder, linear_issue, linear_viewer, now, pr, ts},
    fakes::FakeCache,
    harness::{Harness, config, config_with_linear, linear_workflow, paired_cache},
};

/// When the earlier surveys were made (two hours before `now()`).
const EARLIER: &str = "2026-11-19T10:00:00Z";
const NO_NETWORK: &str = "Linear did not answer within 20s";

fn acme_linear(kind: LinearSourceKind) -> Source {
    Source::Owner {
        owner: "acme".to_owned(),
        kind: SourceKind::Linear(kind),
    }
}

fn failed(error: &str) -> SourceState {
    SourceState::Failed {
        error: error.to_owned(),
    }
}

/// A Linear survey of `acme`: `viewer` (if read), `issues`, and each Linear
/// source's state, dated `observed_at`.
struct Answer {
    viewer: Option<LinearViewer>,
    issues: Vec<LinearIssue>,
    team_states: Vec<LinearTeamStates>,
    states: Vec<(LinearSourceKind, SourceState)>,
    observed_at: Option<&'static str>,
}

impl Answer {
    /// The viewer and `issues`, every source complete.
    fn complete(issues: Vec<LinearIssue>) -> Self {
        Self {
            viewer: Some(linear_viewer("acme", "acme")),
            issues,
            team_states: Vec::new(),
            states: vec![
                (LinearSourceKind::Viewer, SourceState::Complete),
                (LinearSourceKind::AssignedIssues, SourceState::Complete),
            ],
            observed_at: None,
        }
    }

    /// Linear could not be reached: both sources failed, nothing read (as
    /// the adapter reports a timeout on the viewer query).
    fn unreachable() -> Self {
        Self {
            viewer: None,
            issues: Vec::new(),
            team_states: Vec::new(),
            states: vec![
                (LinearSourceKind::Viewer, failed(NO_NETWORK)),
                (LinearSourceKind::AssignedIssues, failed(NO_NETWORK)),
            ],
            observed_at: None,
        }
    }

    fn at(mut self, observed_at: &'static str) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    fn state(mut self, kind: LinearSourceKind, state: SourceState) -> Self {
        self.states.retain(|(k, _)| *k != kind);
        self.states.push((kind, state));
        self
    }

    fn team_states(mut self, team_states: Vec<LinearTeamStates>) -> Self {
        self.team_states = team_states;
        self
    }

    fn viewer(mut self, viewer: LinearViewer) -> Self {
        self.viewer = Some(viewer);
        self
    }

    fn survey(self) -> LinearSurvey {
        let mut coverage = Coverage::default();
        for (kind, state) in self.states {
            coverage.record(acme_linear(kind), state);
        }
        LinearSurvey {
            authentication_failed: false,
            part: LinearPart {
                viewers: self.viewer.into_iter().collect(),
                issues: self.issues,
                team_states: self.team_states,
                observed_at: self.observed_at.map(ts),
            },
            coverage,
        }
    }
}

fn workflow_config() -> den::config::Config {
    config_with_linear(&["acme", "octo"], &[("acme", "acme")])
}

/// A complete GitHub survey of acme and octo, made at `at`, with `prs` open in
/// `acme/api`.
fn github(at: &str, prs: &[u64]) -> Observation {
    let mut builder = ObservationBuilder::new()
        .observed_at(at)
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .repo("acme/api");
    for number in prs {
        builder = builder.pull_request(pr("acme/api", *number));
    }
    builder.build()
}

/// A cache holding an earlier survey with `acme`'s Linear part: the Linear
/// viewer `viewer` and `issues`, every source complete, all read at EARLIER.
fn cached_with_linear(viewer: LinearViewer, issues: Vec<LinearIssue>) -> FakeCache {
    let mut observation = github(EARLIER, &[]);
    observation.linear = LinearPart {
        viewers: vec![viewer],
        issues,
        observed_at: Some(ts(EARLIER)),
        ..LinearPart::default()
    };
    for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
        observation
            .coverage
            .record(acme_linear(kind), SourceState::Complete);
    }
    FakeCache::holding(paired_cache(
        CachedObservation::fresh(observation),
        "acme/api",
    ))
}

/// The identifiers of the Linear rows, in list order.
fn linear_rows(view: &ViewModel) -> Vec<String> {
    view.list
        .rows
        .iter()
        .filter(|r| r.kind == WorkItemKind::LinearIssue)
        .map(|r| r.reference.clone())
        .collect()
}

/// The PR rows' references, in list order.
fn pr_rows(view: &ViewModel) -> Vec<String> {
    view.list
        .rows
        .iter()
        .filter(|r| r.kind == WorkItemKind::PullRequest)
        .map(|r| format!("{}{}", r.repo, r.reference))
        .collect()
}

/// Performs `effect` and everything it leads to.
fn perform_all(harness: &mut Harness, effect: &Effect) {
    for follow_up in harness.perform(effect) {
        perform_all(harness, &follow_up);
    }
}

/// The last Observation den stored.
fn last_stored(harness: &Harness) -> CachedObservation {
    harness
        .cache
        .stored()
        .last()
        .cloned()
        .expect("a stored cache")
}

fn linear_carried(cached: &CachedObservation) -> Vec<&Carried> {
    cached
        .carried
        .iter()
        .filter(|c| c.source.is_linear())
        .collect()
}

fn logged(harness: &Harness, wanted: &str) -> bool {
    harness.effects.iter().any(|e| match e {
        Effect::Log(record) => record.message.contains(wanted),
        _ => false,
    })
}

/// Selects the row whose reference is `reference` and returns the view.
fn select(harness: &mut Harness, reference: &str) -> ViewModel {
    for _ in 0..harness.view.list.rows.len() {
        let selected = harness
            .view
            .detail
            .content
            .as_ref()
            .map(|d| d.reference.clone())
            .unwrap_or_default();
        if selected.contains(reference) {
            return harness.view.clone();
        }
        harness.settle(Event::Key(Key::Down));
    }
    panic!("no row {reference} in {:?}", harness.view.list.rows);
}

#[test]
fn startup_shows_the_cached_linear_part_with_its_age_and_a_refresh_replaces_it() {
    let cache = cached_with_linear(
        linear_viewer("acme", "acme"),
        vec![linear_issue("acme", "id-7", "TEAM-7").build()],
    );
    let mut harness =
        Harness::with_cache(Ok(workflow_config()), cache).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-8", "TEAM-8").build()]).survey(),
    );
    harness.answer(Ok(github(support::build::NOW, &[])));

    let effects = harness.send(Event::Started { now: now() });
    harness.perform(&Effect::LoadCache);
    assert_eq!(linear_rows(&harness.view), ["TEAM-7"], "shown at once");
    let status = &harness.view.header.status;
    assert!(
        status.contains("cached") && status.contains("10:00 UTC") && status.contains("2 h ago"),
        "{status}"
    );
    let detail = select(&mut harness, "TEAM-7")
        .detail
        .content
        .expect("a detail");
    assert!(detail.observed.contains("10:00 UTC"), "{}", detail.observed);

    let linear = effects
        .iter()
        .find(|e| matches!(e, Effect::SurveyLinear { .. }))
        .expect("Linear is surveyed");
    perform_all(&mut harness, linear);
    assert_eq!(
        linear_rows(&harness.view),
        ["TEAM-8"],
        "a complete Linear refresh replaces the cached Linear Issues"
    );
    let github = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .expect("GitHub is surveyed");
    perform_all(&mut harness, github);
    assert_eq!(linear_rows(&harness.view), ["TEAM-8"]);
    let stored = last_stored(&harness);
    assert!(stored.carried.is_empty(), "{:?}", stored.carried);
    assert_eq!(stored.observation.linear.observed_at, Some(now()));
}

#[test]
fn a_failed_linear_refresh_keeps_the_last_good_linear_data_with_its_age() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()])
            .at(EARLIER)
            .survey(),
    );
    // GitHub answered later than Linear: the kept Linear data's age is
    // Linear's own, not the GitHub survey's.
    harness.answer(Ok(github("2026-11-19T11:00:00Z", &[1])));
    harness.linear.push_answer(Answer::unreachable().survey());
    harness.answer(Ok(github(support::build::NOW, &[2])));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    let view = select(&mut harness, "TEAM-7");
    assert_eq!(linear_rows(&view), ["TEAM-7"], "the last good data stays");
    let row = view
        .list
        .rows
        .iter()
        .find(|r| r.reference == "TEAM-7")
        .unwrap();
    assert_eq!(row.step_label, "Implement");
    assert_eq!(
        row.waiting_label, "you",
        "the kept Linear viewer still says whose it is"
    );
    assert!(row.incomplete);
    assert!(!view.header.coverage_complete);
    let coverage = &view.header.coverage;
    assert!(
        coverage.contains(&format!(
            "assigned Linear Issues of acme failed: {NO_NETWORK} (showing data from 10:00 UTC, 2 h ago)"
        )),
        "{coverage}"
    );
    let detail = view.detail.content.expect("a detail");
    assert!(
        detail
            .coverage
            .iter()
            .any(|line| line.contains("failed") && line.contains("showing data from 10:00 UTC")),
        "{:?}",
        detail.coverage
    );
    assert!(detail.observed.contains("10:00 UTC"), "{}", detail.observed);

    let stored = last_stored(&harness);
    let mut kept: Vec<(Source, jiff::Timestamp)> = linear_carried(&stored)
        .iter()
        .map(|c| (c.source.clone(), c.observed_at))
        .collect();
    kept.sort();
    assert_eq!(
        kept,
        [
            (acme_linear(LinearSourceKind::Viewer), ts(EARLIER)),
            (acme_linear(LinearSourceKind::AssignedIssues), ts(EARLIER)),
        ],
        "aged by the Linear survey that read it"
    );
}

#[test]
fn data_kept_by_two_failed_linear_refreshes_keeps_its_original_age() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()])
            .at(EARLIER)
            .survey(),
    );
    harness.answer(Ok(github(EARLIER, &[])));
    for _ in 0..2 {
        harness.linear.push_answer(Answer::unreachable().survey());
        harness.answer(Ok(github(support::build::NOW, &[])));
    }
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));
    harness.settle(Event::Key(Key::Char('r')));

    assert_eq!(linear_rows(&harness.view), ["TEAM-7"]);
    let stored = last_stored(&harness);
    assert!(
        linear_carried(&stored)
            .iter()
            .all(|c| c.observed_at == ts(EARLIER)),
        "{:?}",
        stored.carried
    );
}

#[test]
fn a_failed_linear_refresh_blocks_nothing_needs_you() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness
        .linear
        .push_answer(Answer::complete(Vec::new()).at(EARLIER).survey());
    harness.answer(Ok(github(EARLIER, &[])));
    harness.linear.push_answer(Answer::unreachable().survey());
    harness.answer(Ok(github(support::build::NOW, &[])));
    harness.start();
    assert_eq!(
        harness.view.list.empty.as_ref().expect("empty").title,
        "Nothing in Repo acme/api."
    );

    harness.settle(Event::Key(Key::Char('r')));
    let empty = harness.view.list.empty.clone().expect("still empty");
    assert_ne!(empty.title, "Nothing needs you.");
    assert!(
        harness
            .view
            .header
            .coverage
            .contains("assigned Linear Issues of acme failed")
            && harness
                .view
                .header
                .coverage
                .contains("showing data from 10:00 UTC"),
        "{}",
        harness.view.header.coverage
    );
}

#[test]
fn a_rate_limited_linear_refresh_keeps_the_issues_it_did_not_reach() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![
            linear_issue("acme", "id-7", "TEAM-7").build(),
            linear_issue("acme", "id-8", "TEAM-8")
                .title("Old title")
                .build(),
        ])
        .at(EARLIER)
        .survey(),
    );
    harness.answer(Ok(github(EARLIER, &[])));
    harness.linear.push_answer(
        Answer::complete(vec![
            linear_issue("acme", "id-8", "TEAM-8")
                .title("New title")
                .build(),
        ])
        .state(
            LinearSourceKind::AssignedIssues,
            SourceState::Partial(Incompleteness::LinearRateLimited {
                resets_at: Some(ts("2026-11-19T13:00:00Z")),
            }),
        )
        .survey(),
    );
    harness.answer(Ok(github(support::build::NOW, &[])));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    let mut rows = linear_rows(&harness.view);
    rows.sort();
    assert_eq!(rows, ["TEAM-7", "TEAM-8"], "TEAM-7 was not reached");
    let titles: Vec<&str> = harness
        .view
        .list
        .rows
        .iter()
        .filter(|r| r.reference == "TEAM-8")
        .map(|r| r.title.as_str())
        .collect();
    assert_eq!(titles, ["New title"], "what the refresh read wins");
    assert!(
        harness
            .view
            .header
            .status
            .contains("Linear rate limited until 13:00 UTC"),
        "{}",
        harness.view.header.status
    );
    let stored = last_stored(&harness);
    let kept: Vec<&Source> = linear_carried(&stored).iter().map(|c| &c.source).collect();
    assert_eq!(
        kept,
        [&acme_linear(LinearSourceKind::AssignedIssues)],
        "the viewer was read again: only the issue list is carried"
    );
}

/// Each Linear source replaces only the facts it listed: a start team's list
/// failing keeps its issues while a complete assigned list drops its own.
#[test]
fn a_linear_source_read_completely_replaces_only_its_own_facts() {
    let start_team = |id: &str, identifier: &str| {
        linear_issue("acme", id, identifier)
            .unassigned()
            .listed_by(LinearSourceKind::StartTeamIssues)
            .build()
    };
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![
            linear_issue("acme", "id-7", "TEAM-7").build(),
            start_team("id-9", "TEAM-9"),
        ])
        .state(LinearSourceKind::StartTeamIssues, SourceState::Complete)
        .at(EARLIER)
        .survey(),
    );
    harness.answer(Ok(github(EARLIER, &[])));
    harness.linear.push_answer(
        Answer::complete(Vec::new())
            .state(LinearSourceKind::StartTeamIssues, failed(NO_NETWORK))
            .survey(),
    );
    harness.answer(Ok(github(support::build::NOW, &[])));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    let stored = last_stored(&harness);
    let identifiers: Vec<&str> = stored
        .observation
        .linear
        .issues
        .iter()
        .map(|i| i.identifier.as_str())
        .collect();
    assert_eq!(
        identifiers,
        ["TEAM-9"],
        "TEAM-7 is no longer assigned; TEAM-9's list was not read"
    );
    let kept: Vec<(&Source, jiff::Timestamp)> = linear_carried(&stored)
        .iter()
        .map(|c| (&c.source, c.observed_at))
        .collect();
    assert_eq!(
        kept,
        [(&acme_linear(LinearSourceKind::StartTeamIssues), ts(EARLIER))]
    );
}

/// acme's Linear Workflow with start team `TEAM` (#33), so its Linear survey
/// reads all four Linear sources.
fn start_team_config() -> den::config::Config {
    den::config::Config {
        linear: vec![den::config::LinearWorkflow {
            start_teams: vec!["TEAM".to_owned()],
            ..linear_workflow("acme", "acme")
        }],
        ..config(&["acme", "octo"])
    }
}

/// `TEAM`'s workflow states, as acme's workflow read them.
fn team_states() -> LinearTeamStates {
    LinearTeamStates {
        owner: "acme".to_owned(),
        workspace: "acme".to_owned(),
        team: LinearTeam {
            id: "team-team".to_owned(),
            key: "TEAM".to_owned(),
        },
        name: "Team".to_owned(),
        states: vec![LinearState {
            id: "TEAM-todo".to_owned(),
            name: "Todo".to_owned(),
            kind: LinearStateKind::Unstarted,
        }],
    }
}

/// Integration (#33 + #34): a failed start-team refresh keeps the last good
/// Start items, still Start and Waiting On nobody, with their own age, and
/// marked incomplete; the team states a failed team-states source didn't
/// re-read are kept too. Only the assigned list, read completely, replaces
/// its own facts.
#[test]
fn a_failed_start_team_refresh_keeps_the_last_good_start_items_with_their_age() {
    let all_complete = |answer: Answer| {
        answer
            .state(LinearSourceKind::StartTeamIssues, SourceState::Complete)
            .state(LinearSourceKind::TeamStates, SourceState::Complete)
    };
    let mut harness = Harness::new(Ok(start_team_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        all_complete(Answer::complete(vec![
            linear_issue("acme", "id-9", "TEAM-9")
                .up_for_grabs()
                .listed_by(LinearSourceKind::StartTeamIssues)
                .build(),
        ]))
        .team_states(vec![team_states()])
        .at(EARLIER)
        .survey(),
    );
    harness.answer(Ok(github("2026-11-19T11:00:00Z", &[])));
    harness.linear.push_answer(
        Answer::complete(Vec::new())
            .state(LinearSourceKind::StartTeamIssues, failed(NO_NETWORK))
            .state(LinearSourceKind::TeamStates, failed(NO_NETWORK))
            .survey(),
    );
    harness.answer(Ok(github(support::build::NOW, &[])));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    let view = select(&mut harness, "TEAM-9");
    assert_eq!(linear_rows(&view), ["TEAM-9"], "the last good Start stays");
    let row = view
        .list
        .rows
        .iter()
        .find(|r| r.reference == "TEAM-9")
        .unwrap();
    assert_eq!(row.step_label, "Start");
    assert!(row.incomplete, "its source failed");
    let coverage = &view.header.coverage;
    assert!(
        coverage.contains(&format!(
            "start-team Linear Issues of acme failed: {NO_NETWORK} (showing data from 10:00 UTC, 2 h ago)"
        )),
        "{coverage}"
    );
    let detail = view.detail.content.expect("a detail");
    assert_eq!(detail.next_step, "Start");
    assert!(detail.observed.contains("10:00 UTC"), "{}", detail.observed);

    let stored = last_stored(&harness);
    let decisions = den::decide::decide(
        &stored.observation,
        &den::domain::Viewers::of(&stored.observation),
        &start_team_config().policies(),
        support::build::now(),
    );
    let start = decisions
        .iter()
        .find(|d| d.item.identifier.as_deref() == Some("TEAM-9"))
        .expect("TEAM-9 decided");
    assert_eq!(start.next_step, Some(NextStep::Start));
    assert_eq!(start.waiting_on, WaitingOn::Nobody);
    assert_eq!(stored.observation.linear.team_states, [team_states()]);
    let mut kept: Vec<(Source, jiff::Timestamp)> = linear_carried(&stored)
        .iter()
        .map(|c| (c.source.clone(), c.observed_at))
        .collect();
    kept.sort();
    assert_eq!(
        kept,
        [
            (acme_linear(LinearSourceKind::StartTeamIssues), ts(EARLIER)),
            (acme_linear(LinearSourceKind::TeamStates), ts(EARLIER)),
        ],
        "aged by the Linear survey that read them, not the later GitHub survey"
    );
}

#[test]
fn a_changed_linear_viewer_drops_cached_linear_data() {
    let cache = cached_with_linear(
        linear_viewer("acme", "acme"),
        vec![linear_issue("acme", "id-7", "TEAM-7").build()],
    );
    let mut harness =
        Harness::with_cache(Ok(workflow_config()), cache).with_linear_pairing("acme/api", "acme");
    // Another account's key: its viewer reads, its issues don't. Without the
    // viewer check, TEAM-7 would be kept as the last good data.
    let someone_else = LinearViewer {
        id: "user-other".to_owned(),
        name: "Someone Else".to_owned(),
        ..linear_viewer("acme", "acme")
    };
    harness.linear.push_answer(
        Answer::complete(Vec::new())
            .viewer(someone_else)
            .state(LinearSourceKind::AssignedIssues, failed(NO_NETWORK))
            .survey(),
    );
    harness.answer(Ok(github(support::build::NOW, &[])));
    let effects = harness.send(Event::Started { now: now() });
    harness.perform(&Effect::LoadCache);
    assert_eq!(linear_rows(&harness.view), ["TEAM-7"]);

    let linear = effects
        .iter()
        .find(|e| matches!(e, Effect::SurveyLinear { .. }))
        .unwrap();
    perform_all(&mut harness, linear);
    assert!(
        linear_rows(&harness.view).is_empty(),
        "{:?}",
        harness.rows()
    );
    assert!(
        harness.app.linear_binding().is_none(),
        "a changed viewer requires connection setup again"
    );
    let coverage = &harness.view.header.coverage;
    assert!(
        !coverage.contains("assigned Linear Issues"),
        "revoked Linear access contributes no coverage: {coverage}"
    );
    // Changed with #45's save-once rule: the same survey's GitHub answer is
    // still to come, so Linear's answer is saved with it, not on its own.
    assert!(
        harness.cache.stored().is_empty(),
        "saved before GitHub answered"
    );
    let github = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .unwrap();
    perform_all(&mut harness, github);
    assert_eq!(harness.cache.stored().len(), 1, "one save for the survey");
    let stored = last_stored(&harness);
    assert!(linear_carried(&stored).is_empty(), "{:?}", stored.carried);
    let viewers: Vec<&str> = stored
        .observation
        .linear
        .viewers
        .iter()
        .map(|v| v.id.as_str())
        .collect();
    // The previous pairing cannot authorize a different viewer.
    assert!(viewers.is_empty());
}

#[test]
fn a_linear_viewer_changing_within_a_session_drops_the_earlier_linear_data() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()]).survey(),
    );
    harness.answer(Ok(github(support::build::NOW, &[])));
    let other_workspace = linear_viewer("acme", "acme-two");
    harness.linear.push_answer(
        Answer::complete(Vec::new())
            .viewer(other_workspace)
            .state(
                LinearSourceKind::AssignedIssues,
                SourceState::Partial(Incompleteness::LinearRateLimited { resets_at: None }),
            )
            .survey(),
    );
    harness.answer(Ok(github(support::build::NOW, &[])));
    harness.start();
    assert_eq!(linear_rows(&harness.view), ["TEAM-7"]);
    harness.settle(Event::Key(Key::Char('r')));
    assert!(
        linear_rows(&harness.view).is_empty(),
        "{:?}",
        harness.rows()
    );
}

/// The config is fixed for a session, so a cache read with another Linear
/// Workflow setup is fitted when it loads.
#[test]
fn a_cached_linear_part_is_dropped_when_its_owner_s_linear_workflow_changed_or_went() {
    for (case, config) in [
        (
            "another workspace",
            config_with_linear(&["acme", "octo"], &[("acme", "acme-two")]),
        ),
        ("no Linear Workflow", config(&["acme", "octo"])),
    ] {
        let cache = cached_with_linear(
            linear_viewer("acme", "acme"),
            vec![linear_issue("acme", "id-7", "TEAM-7").build()],
        );
        let mut harness = Harness::with_cache(Ok(config), cache);
        harness.send(Event::Started { now: now() });
        harness.perform(&Effect::LoadCache);
        assert!(
            linear_rows(&harness.view).is_empty(),
            "{case}: {:?}",
            harness.rows()
        );
        assert!(
            !harness.view.header.coverage.contains("showing data"),
            "{case}: {}",
            harness.view.header.coverage
        );
    }
}

#[test]
fn a_linear_failure_never_discards_fresh_github_data() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()])
            .at(EARLIER)
            .survey(),
    );
    harness.answer(Ok(github(EARLIER, &[1])));
    harness.linear.push_answer(Answer::unreachable().survey());
    harness.answer(Ok(github(support::build::NOW, &[2])));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    assert_eq!(
        pr_rows(&harness.view),
        ["acme/api#2"],
        "GitHub's refresh wins"
    );
    assert_eq!(linear_rows(&harness.view), ["TEAM-7"]);
    let stored = last_stored(&harness);
    assert!(
        stored.carried.iter().all(|c| c.source.is_linear()),
        "no GitHub source is carried: {:?}",
        stored.carried
    );
    assert_eq!(stored.observation.observed_at, now());
}

#[test]
fn a_github_failure_never_discards_fresh_linear_data() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()])
            .at(EARLIER)
            .survey(),
    );
    harness.answer(Ok(github(EARLIER, &[1])));
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-8", "TEAM-8").build()]).survey(),
    );
    // acme's Pull Requests fail; everything else is read.
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .repo("acme/api")
        .owner_coverage("acme", SourceKind::PullRequests, failed("timeout"))
        .build()));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    assert_eq!(
        linear_rows(&harness.view),
        ["TEAM-8"],
        "Linear's refresh wins"
    );
    assert_eq!(pr_rows(&harness.view), ["acme/api#1"], "kept with its age");
    let stored = last_stored(&harness);
    assert!(
        linear_carried(&stored).is_empty(),
        "no Linear source is carried: {:?}",
        stored.carried
    );
    assert!(
        stored.carried.iter().any(|c| c.source
            == Source::Owner {
                owner: "acme".to_owned(),
                kind: SourceKind::PullRequests
            }
            && c.observed_at == ts(EARLIER)),
        "{:?}",
        stored.carried
    );
}

#[test]
fn a_github_survey_that_fails_outright_still_shows_the_linear_refresh() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()]).survey(),
    );
    harness.answer(Ok(github(EARLIER, &[1])));
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-8", "TEAM-8").build()]).survey(),
    );
    harness.answer(Err(SurveyFailure::GhNotInstalled));
    harness.start();
    harness.settle(Event::Key(Key::Char('r')));

    assert_eq!(linear_rows(&harness.view), ["TEAM-8"]);
    assert_eq!(pr_rows(&harness.view), ["acme/api#1"]);
}

/// A GitHub answer for another viewer than the cache's starts afresh (the
/// cache is per viewer), but keeps this session's Linear answer.
#[test]
fn a_github_survey_of_another_viewer_keeps_this_session_s_linear_answer() {
    let mut observation = ObservationBuilder::new()
        .viewer("someone")
        .observed_at(EARLIER)
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .build();
    observation.linear.viewers = vec![linear_viewer("acme", "acme")];
    observation.linear.issues = vec![linear_issue("acme", "id-7", "TEAM-7").build()];
    let cache = FakeCache::holding(paired_cache(
        CachedObservation::fresh(observation),
        "acme/api",
    ));
    let mut harness =
        Harness::with_cache(Ok(workflow_config()), cache).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-8", "TEAM-8").build()]).survey(),
    );
    harness.answer(Ok(github(support::build::NOW, &[])));
    harness.start();

    assert_eq!(linear_rows(&harness.view), ["TEAM-8"]);
    assert!(
        harness.view.header.coverage_complete,
        "{}",
        harness.view.header.coverage
    );
}

/// A start-team Linear Issue, as #33's start-team source lists it.
fn start_team_issue(id: &str, identifier: &str) -> LinearIssue {
    linear_issue("acme", id, identifier)
        .unassigned()
        .listed_by(LinearSourceKind::StartTeamIssues)
        .build()
}

/// A Linear source den no longer reads (its config stopped asking for it)
/// goes with its facts: it is neither "not yet fetched" forever nor are its
/// Linear Issues kept forever, so "Nothing needs you" is reachable again.
#[test]
fn a_linear_source_no_longer_surveyed_is_dropped_with_its_facts() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![start_team_issue("id-9", "TEAM-9")])
            .state(LinearSourceKind::StartTeamIssues, SourceState::Complete)
            .at(EARLIER)
            .survey(),
    );
    harness.answer(Ok(github(EARLIER, &[])));
    // The next survey reads only what this config asks for.
    harness
        .linear
        .push_answer(Answer::complete(Vec::new()).survey());
    harness.answer(Ok(github(support::build::NOW, &[])));
    harness.start();
    // On #34 alone TEAM-9 showed as a row here. Merged with #33, an unassigned
    // Linear Issue the workflow's Start policy doesn't offer (this config
    // names no start team) gets no Decision, so it has no row; what this test
    // is about is whether den still holds the fact, so it reads the save.
    let identifiers = |stored: &CachedObservation| -> Vec<String> {
        stored
            .observation
            .linear
            .issues
            .iter()
            .map(|i| i.identifier.clone())
            .collect()
    };
    assert_eq!(identifiers(&last_stored(&harness)), ["TEAM-9"]);
    harness.settle(Event::Key(Key::Char('r')));

    assert!(
        linear_rows(&harness.view).is_empty(),
        "{:?}",
        harness.rows()
    );
    let stored = last_stored(&harness);
    assert!(
        identifiers(&stored).is_empty(),
        "{:?}",
        identifiers(&stored)
    );
    let start_team = acme_linear(LinearSourceKind::StartTeamIssues);
    assert!(
        stored.observation.coverage.get(&start_team).is_none(),
        "{:?}",
        stored.observation.coverage
    );
    assert_eq!(
        harness.view.list.empty.as_ref().expect("empty").title,
        "Nothing in Repo acme/api."
    );
}

/// The same for a cache written under a config that read more Linear
/// sources: what this config doesn't ask for is dropped when it loads, and a
/// cached Linear Issue whose source was never recorded is not kept forever
/// behind a complete coverage.
#[test]
fn a_cached_linear_source_this_config_does_not_read_is_dropped_on_load() {
    for (case, recorded) in [("recorded", true), ("never recorded", false)] {
        let mut observation = github(EARLIER, &[]);
        observation.linear = LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues: vec![start_team_issue("id-9", "TEAM-9")],
            observed_at: Some(ts(EARLIER)),
            ..LinearPart::default()
        };
        for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
            observation
                .coverage
                .record(acme_linear(kind), SourceState::Complete);
        }
        if recorded {
            observation.coverage.record(
                acme_linear(LinearSourceKind::StartTeamIssues),
                SourceState::Complete,
            );
        }
        let cache = FakeCache::holding(paired_cache(
            CachedObservation::fresh(observation),
            "acme/api",
        ));
        let mut harness = Harness::with_cache(Ok(workflow_config()), cache)
            .with_linear_pairing("acme/api", "acme");
        harness
            .linear
            .push_answer(Answer::complete(Vec::new()).survey());
        harness.answer(Ok(github(support::build::NOW, &[])));
        let effects = harness.send(Event::Started { now: now() });
        harness.perform(&Effect::LoadCache);
        assert!(
            linear_rows(&harness.view).is_empty(),
            "{case}: dropped on load: {:?}",
            harness.rows()
        );

        for effect in &effects {
            perform_all(&mut harness, effect);
        }
        assert!(
            linear_rows(&harness.view).is_empty(),
            "{case}: {:?}",
            harness.rows()
        );
        let stored = last_stored(&harness);
        assert!(
            stored
                .observation
                .coverage
                .get(&acme_linear(LinearSourceKind::StartTeamIssues))
                .is_none(),
            "{case}: {:?}",
            stored.observation.coverage
        );
        assert_eq!(
            harness.view.list.empty.as_ref().expect("empty").title,
            "Nothing in Repo acme/api.",
            "{case}"
        );
    }
}

/// A Linear Issue read completely is dated by the Linear survey that read
/// it, never by the GitHub survey, whichever of the two is newer.
#[test]
fn a_linear_issue_shows_when_linear_read_it() {
    for (case, linear_at, github_at, shown) in [
        (
            "Linear older",
            Some(EARLIER),
            support::build::NOW,
            "10:00 UTC",
        ),
        ("GitHub older", None, EARLIER, "12:00 UTC"),
    ] {
        let mut harness =
            Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
        let mut answer = Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()]);
        if let Some(at) = linear_at {
            answer = answer.at(at);
        }
        harness.linear.push_answer(answer.survey());
        harness.answer(Ok(github(github_at, &[])));
        harness.start();

        let detail = select(&mut harness, "TEAM-7")
            .detail
            .content
            .expect("a detail");
        assert!(
            detail.observed.starts_with(shown),
            "{case}: {}",
            detail.observed
        );
    }
}

/// The Linear answer can arrive before the cache loads: a changed Linear
/// viewer still drops the cached Linear data, and says so.
#[test]
fn a_changed_linear_viewer_found_when_the_cache_loads_is_logged() {
    let cache = cached_with_linear(
        linear_viewer("acme", "acme"),
        vec![linear_issue("acme", "id-7", "TEAM-7").build()],
    );
    let mut harness =
        Harness::with_cache(Ok(workflow_config()), cache).with_linear_pairing("acme/api", "acme");
    let someone_else = LinearViewer {
        id: "user-other".to_owned(),
        ..linear_viewer("acme", "acme")
    };
    harness.linear.push_answer(
        Answer::complete(Vec::new())
            .viewer(someone_else)
            .state(LinearSourceKind::AssignedIssues, failed(NO_NETWORK))
            .survey(),
    );
    let effects = harness.send(Event::Started { now: now() });
    let linear = effects
        .iter()
        .find(|e| matches!(e, Effect::SurveyLinear { .. }))
        .unwrap();
    perform_all(&mut harness, linear);
    harness.perform(&Effect::LoadCache);

    assert!(
        linear_rows(&harness.view).is_empty(),
        "{:?}",
        harness.rows()
    );
    assert!(logged(&harness, "Linear authentication changed"));
}

/// A Linear answer logs the Linear data it kept, not GitHub's again.
#[test]
fn a_linear_answer_logs_only_the_linear_data_it_kept() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()])
            .at(EARLIER)
            .survey(),
    );
    harness.answer(Ok(github(EARLIER, &[1])));
    harness.linear.push_answer(Answer::unreachable().survey());
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .fully_read_owner("octo")
        .repo("acme/api")
        .owner_coverage("acme", SourceKind::PullRequests, failed("timeout"))
        .build()));
    harness.start();
    let effects = harness.send(Event::Key(Key::Char('r')));
    let github = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .unwrap();
    perform_all(&mut harness, github);
    assert!(logged(
        &harness,
        "kept earlier data for: Pull Requests of acme"
    ));
    let linear = effects
        .iter()
        .find(|e| matches!(e, Effect::SurveyLinear { .. }))
        .unwrap();
    let linear_log = harness.perform(linear);
    let kept: Vec<&str> = linear_log
        .iter()
        .filter_map(|e| match e {
            Effect::Log(record) if record.message.starts_with("kept earlier data") => {
                Some(record.message.as_str())
            }
            _ => None,
        })
        .collect();
    assert!(!kept.is_empty(), "the Linear data kept is logged");
    assert!(
        kept.iter()
            .all(|m| m.contains("Linear") && !m.contains("Pull Requests")),
        "{kept:?}"
    );
}

/// GitHub recovering after the Linear-only stand-in keeps the Linear data
/// shown, data kept by a failed Linear refresh and its age included. The
/// stand-in has no GitHub viewer, so the GitHub answer starts afresh, but a
/// GitHub answer never touches the Linear part.
#[test]
fn linear_data_kept_on_the_linear_only_stand_in_survives_github_s_recovery() {
    let mut harness = Harness::new(Ok(workflow_config())).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(
        Answer::complete(vec![linear_issue("acme", "id-7", "TEAM-7").build()])
            .at(EARLIER)
            .survey(),
    );
    harness.answer(Err(SurveyFailure::GhNotInstalled));
    harness.linear.push_answer(Answer::unreachable().survey());
    harness.answer(Ok(github(support::build::NOW, &[1])));
    harness.start();
    assert_eq!(linear_rows(&harness.view), ["TEAM-7"], "the stand-in");
    assert!(
        harness.cache.stored().is_empty(),
        "the stand-in is never saved"
    );

    harness.settle(Event::Key(Key::Char('r')));

    assert_eq!(
        linear_rows(&harness.view),
        ["TEAM-7"],
        "kept by the failed Linear refresh, and still kept when GitHub answers"
    );
    assert_eq!(pr_rows(&harness.view), ["acme/api#1"]);
    let stored = last_stored(&harness);
    let saved: Vec<&str> = stored
        .observation
        .linear
        .issues
        .iter()
        .map(|i| i.identifier.as_str())
        .collect();
    assert_eq!(saved, ["TEAM-7"]);
    let mut kept: Vec<(Source, jiff::Timestamp)> = linear_carried(&stored)
        .iter()
        .map(|c| (c.source.clone(), c.observed_at))
        .collect();
    kept.sort();
    assert_eq!(
        kept,
        [
            (acme_linear(LinearSourceKind::Viewer), ts(EARLIER)),
            (acme_linear(LinearSourceKind::AssignedIssues), ts(EARLIER)),
        ],
        "with the age of the Linear survey that read it"
    );
}

/// A Linear answer that arrives before the cache loads logs the cached
/// Linear data it kept, as one arriving later does.
#[test]
fn linear_data_kept_from_the_cache_is_logged_when_linear_answered_first() {
    let cache = cached_with_linear(
        linear_viewer("acme", "acme"),
        vec![linear_issue("acme", "id-7", "TEAM-7").build()],
    );
    let mut harness =
        Harness::with_cache(Ok(workflow_config()), cache).with_linear_pairing("acme/api", "acme");
    harness.linear.push_answer(Answer::unreachable().survey());
    let effects = harness.send(Event::Started { now: now() });
    let linear = effects
        .iter()
        .find(|e| matches!(e, Effect::SurveyLinear { .. }))
        .unwrap();
    perform_all(&mut harness, linear);
    harness.perform(&Effect::LoadCache);

    assert_eq!(linear_rows(&harness.view), ["TEAM-7"]);
    assert!(
        logged(
            &harness,
            "kept earlier data for: Linear viewer of acme from 2026-11-19T10:00:00Z; \
             assigned Linear Issues of acme from 2026-11-19T10:00:00Z"
        ),
        "{:?}",
        harness.effects
    );
}

/// A Linear survey den couldn't queue before it knew the time has no date.
/// Merged into the cache, it leaves the Linear part's date as it was, so the
/// cached Linear Issue is still dated by the Linear survey that read it, not
/// by the GitHub survey.
#[test]
fn an_undated_linear_answer_leaves_the_cached_linear_date() {
    let mut observation = github("2026-11-19T11:00:00Z", &[]);
    observation.linear = LinearPart {
        viewers: vec![linear_viewer("acme", "acme")],
        issues: vec![linear_issue("acme", "id-7", "TEAM-7").build()],
        observed_at: Some(ts(EARLIER)),
        ..LinearPart::default()
    };
    for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
        observation
            .coverage
            .record(acme_linear(kind), SourceState::Complete);
    }
    let cache = FakeCache::holding(paired_cache(
        CachedObservation::fresh(observation),
        "acme/api",
    ));
    let mut harness =
        Harness::with_cache(Ok(workflow_config()), cache).with_linear_pairing("acme/api", "acme");
    harness.answer(Ok(github(support::build::NOW, &[])));
    // `r` before any event that carries the time.
    let effects = harness.send(Event::Key(Key::Char('r')));
    let linear = effects
        .iter()
        .find(|e| matches!(e, Effect::SurveyLinear { .. }))
        .cloned()
        .unwrap();
    harness.send(Event::EffectRejected {
        effect: Box::new(linear),
        reason: "the Linear queue is full".to_owned(),
    });
    harness.perform(&Effect::LoadCache);

    let detail = select(&mut harness, "TEAM-7")
        .detail
        .content
        .expect("a detail");
    assert!(detail.observed.contains("10:00 UTC"), "{}", detail.observed);

    let github = effects
        .iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .unwrap();
    perform_all(&mut harness, github);
    assert_eq!(
        last_stored(&harness).observation.linear.observed_at,
        Some(ts(EARLIER)),
        "the Linear data den holds was read at 10:00"
    );
}

/// `merge_linear` takes "no longer read" from the config, never from a
/// survey's silence: a Linear source the config reads that the survey has no
/// word on is not yet fetched, and its earlier data is kept with its age,
/// whoever built the survey.
#[test]
fn a_linear_source_the_config_reads_is_kept_when_a_survey_says_nothing_of_it() {
    let mut observation = github(EARLIER, &[]);
    observation.linear = LinearPart {
        viewers: vec![linear_viewer("acme", "acme")],
        issues: vec![linear_issue("acme", "id-7", "TEAM-7").build()],
        observed_at: Some(ts(EARLIER)),
        ..LinearPart::default()
    };
    for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
        observation
            .coverage
            .record(acme_linear(kind), SourceState::Complete);
    }
    let assigned = acme_linear(LinearSourceKind::AssignedIssues);
    let mut survey = Answer::complete(Vec::new())
        .at(support::build::NOW)
        .survey();
    survey.coverage.retain(|e| e.source != assigned);
    let reads = [LinearReads {
        owner: "acme",
        workspace: "acme",
        sources: vec![LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues],
    }];

    let merged = merge_linear(CachedObservation::fresh(observation), survey, &reads).cached;

    let issues: Vec<&str> = merged
        .observation
        .linear
        .issues
        .iter()
        .map(|i| i.identifier.as_str())
        .collect();
    assert_eq!(issues, ["TEAM-7"]);
    assert_eq!(
        merged.observation.coverage.get(&assigned),
        Some(&SourceState::NotYetFetched)
    );
    assert_eq!(
        merged.carried,
        [Carried {
            source: assigned,
            observed_at: ts(EARLIER),
        }]
    );
}
