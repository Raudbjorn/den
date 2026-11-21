//! Seam 2, Linear writes (#38, ADR-0004): the Start / Implement sheet and the
//! Mark for release sheet. den's only remote writes are assigning a Linear
//! Issue to the Linear viewer and moving its state, each only after `Enter`
//! on a sheet that showed it; before either, den re-reads the issue and asks
//! again if it changed. A write stops at the first failure and is never
//! undone; an unknown outcome is followed by a re-read, never a retry.
//!
//! Every write here goes to a fake Linear writer that records it: nothing
//! reaches Linear.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Event, Key,
        view_model::{LaunchSheet, LaunchStatus, Overlay, SheetMessage, Tone},
    },
    config::{Config, LinearWorkflow},
    domain::{
        Coverage, LinearPart, LinearSourceKind, NextStep, RepoId, Source, SourceKind, SourceState,
        facts::{
            LinearIssue, LinearState, LinearStateKind, LinearUser, MemberPullRequestState,
            PullRequestRef, PullRequestState,
        },
    },
    handoff::{BaseRef, BranchStart, LaunchPlan},
    jobs::{JobRecord, JobSubject},
    ports::{LinearIssueNow, LinearIssueRead, LinearSurvey, LinearWriteOutcome, MemberStatesRead},
};
use std::{collections::BTreeMap, path::PathBuf};
use support::{
    build::{
        LINEAR_ME_ID, LINEAR_ME_NAME, ME, ObservationBuilder, issue, linear_issue, linear_viewer,
    },
    fakes::LinearWriteCall,
    harness::{Harness, linear_workflow},
};

const ROOT: &str = "/home/me/dev";
const API_CLONE: &str = "/home/me/dev/api";
const WEB_CLONE: &str = "/home/me/dev/web";
const OCTO_CLONE: &str = "/home/me/dev/app";
const MAIN_TIP: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const IN_PROGRESS: &str = "In Progress";
const RELEASE: &str = "Ready for Release";
/// Linear's suggested branch for TEAM-8: a safe name, used as it is.
const TEAM_8_BRANCH: &str = "me/team-8-pick-me";

fn todo() -> LinearState {
    LinearState {
        id: "state-todo".to_owned(),
        name: "Todo".to_owned(),
        kind: LinearStateKind::Unstarted,
    }
}

fn in_progress() -> LinearState {
    LinearState {
        id: "state-started".to_owned(),
        name: IN_PROGRESS.to_owned(),
        kind: LinearStateKind::Started,
    }
}

fn release() -> LinearState {
    LinearState {
        id: "state-release".to_owned(),
        name: RELEASE.to_owned(),
        kind: LinearStateKind::Started,
    }
}

fn me() -> LinearUser {
    LinearUser {
        id: LINEAR_ME_ID.to_owned(),
        name: LINEAR_ME_NAME.to_owned(),
    }
}

/// acme's Linear Workflow: start team TEAM, new work in acme/api (and, when
/// `two_repos`, acme/web), in-progress and release states named.
fn workflow(two_repos: bool) -> LinearWorkflow {
    let mut workflow = linear_workflow("acme", "acme");
    workflow.start_teams = vec!["TEAM".to_owned()];
    workflow.new_work_repos = vec![RepoId::new("acme", "api")];
    if two_repos {
        workflow.new_work_repos.push(RepoId::new("acme", "web"));
    }
    workflow.in_progress_state = Some(IN_PROGRESS.to_owned());
    workflow.release_state = Some(RELEASE.to_owned());
    workflow
}

fn config(workflow: LinearWorkflow) -> Config {
    Config {
        owners: vec!["acme".to_owned(), "octo".to_owned()],
        main_clone_root: Some(PathBuf::from(ROOT)),
        main_clones: BTreeMap::new(),
        linear: vec![workflow],
        skips: den::domain::Skips::default(),
    }
}

/// TEAM-8: offered to Start (unassigned, in Todo), with Linear's branch name.
fn team_8() -> LinearIssue {
    linear_issue("acme", "id-8", "TEAM-8")
        .up_for_grabs()
        .state_id("state-todo")
        .title("Pick me")
        .branch_name(TEAM_8_BRANCH)
        .build()
}

/// TEAM-6: assigned to me and still in Todo: Implement, offering the move.
fn team_6() -> LinearIssue {
    linear_issue("acme", "id-6", "TEAM-6")
        .state("Todo", LinearStateKind::Unstarted)
        .state_id("state-todo")
        .title("SECRET TITLE TEXT")
        .build()
}

/// TEAM-7: assigned to me and started: Implement, with nothing to write.
fn team_7() -> LinearIssue {
    linear_issue("acme", "id-7", "TEAM-7")
        .state(IN_PROGRESS, LinearStateKind::Started)
        .build()
}

/// TEAM-3: its only member, acme/api #1, merged: Mark for release.
fn team_3() -> LinearIssue {
    linear_issue("acme", "id-3", "TEAM-3")
        .state(IN_PROGRESS, LinearStateKind::Started)
        .attached("acme/api", 1)
        .build()
}

fn linear_answer() -> LinearSurvey {
    let mut coverage = Coverage::default();
    for kind in [
        LinearSourceKind::Viewer,
        LinearSourceKind::AssignedIssues,
        LinearSourceKind::TeamStates,
        LinearSourceKind::StartTeamIssues,
    ] {
        coverage.record(
            Source::Owner {
                owner: "acme".to_owned(),
                kind: SourceKind::Linear(kind),
            },
            SourceState::Complete,
        );
    }
    LinearSurvey {
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues: vec![team_8(), team_6(), team_7(), team_3()],
            ..LinearPart::default()
        },
        coverage,
    }
}

fn merged_api_1() -> MemberStatesRead {
    let mut coverage = Coverage::default();
    coverage.record(
        Source::Owner {
            owner: "acme".to_owned(),
            kind: SourceKind::MemberStates,
        },
        SourceState::Complete,
    );
    MemberStatesRead {
        states: vec![MemberPullRequestState {
            pr: PullRequestRef::new(RepoId::new("acme", "api"), 1),
            state: PullRequestState::Merged { merged_at: None },
        }],
        // The fake fills in every member asked.
        asked: Vec::new(),
        coverage,
    }
}

/// den started on acme (with its Linear Workflow) and octo (without), both
/// surveys answered, and a Main Clone of each of acme's Repos at hand.
fn started(workflow: LinearWorkflow) -> Harness {
    let mut harness = Harness::new(Ok(config(workflow)));
    queue_round(&harness);
    for (clone, url) in [
        (API_CLONE, "git@github.com:acme/api.git"),
        (WEB_CLONE, "git@github.com:acme/web.git"),
        (OCTO_CLONE, "git@github.com:octo/app.git"),
    ] {
        harness.workspace.add_clone(clone, &[("origin", url)]);
        harness
            .workspace
            .set_remote_head(clone, "origin", "main", MAIN_TIP);
    }
    harness.start();
    harness
}

fn queue_round(harness: &Harness) {
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .repo("acme/web")
        .fully_read_owner("octo")
        .repo("octo/app")
        .issue(issue("octo/app", 9).title("Add a login page").assignee(ME))
        .build()));
    harness.linear.push_answer(linear_answer());
    harness.github.push_member_states(merged_api_1());
}

/// TEAM-8's or another issue's re-read: as given.
fn reread(assignee: Option<LinearUser>, state: LinearState) -> LinearIssueRead {
    LinearIssueRead::Read(LinearIssueNow {
        assignee,
        state,
        team_states: vec![todo(), in_progress(), release()],
        states_complete: true,
    })
}

fn done(assignee: Option<LinearUser>, state: LinearState) -> LinearWriteOutcome {
    LinearWriteOutcome::Done { assignee, state }
}

/// Selects the row whose reference is `reference` and presses `c`.
fn open(harness: &mut Harness, reference: &str) {
    select(harness, reference);
    harness.settle(Event::Key(Key::Char('c')));
}

fn select(harness: &mut Harness, reference: &str) {
    let index = harness
        .view
        .list
        .rows
        .iter()
        .position(|r| r.reference == reference)
        .unwrap_or_else(|| panic!("{reference} is listed: {:?}", harness.rows()));
    while harness.view.list.selected != Some(index) {
        let before = harness.view.list.selected;
        let key = if before.is_some_and(|s| s > index) {
            Key::Up
        } else {
            Key::Down
        };
        harness.settle(Event::Key(key));
        assert_ne!(before, harness.view.list.selected, "selection stuck");
    }
}

fn sheet(harness: &Harness) -> &LaunchSheet {
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => sheet,
        other => panic!("no sheet: {other:?}"),
    }
}

fn messages(sheet: &LaunchSheet) -> String {
    sheet
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn field<'a>(sheet: &'a LaunchSheet, label: &str) -> (&'a str, Option<&'a str>) {
    let field = sheet
        .fields
        .iter()
        .find(|f| f.label == label)
        .unwrap_or_else(|| panic!("no {label} field: {:?}", sheet.fields));
    (field.value.as_str(), field.note.as_deref())
}

fn is_linear(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::ReadLinearIssue { .. }
            | Effect::AssignLinearIssue { .. }
            | Effect::MoveLinearIssue { .. }
    )
}

fn is_linear_write(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::AssignLinearIssue { .. } | Effect::MoveLinearIssue { .. }
    )
}

/// The Effects that do something, in order: Linear's and launches (no logs,
/// no Job checks).
fn acts(effects: &[Effect]) -> Vec<&'static str> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::ReadLinearIssue { .. } => Some("re-read"),
            Effect::AssignLinearIssue { .. } => Some("assign"),
            Effect::MoveLinearIssue { .. } => Some("move"),
            Effect::InspectLaunch { .. } => Some("inspect"),
            Effect::Launch { .. } => Some("launch"),
            _ => None,
        })
        .collect()
}

fn launch_plans(harness: &Harness) -> Vec<LaunchPlan> {
    harness
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Launch { plan, .. } => Some((**plan).clone()),
            _ => None,
        })
        .collect()
}

/// Presses Enter and returns the Effects it led to, performed through the
/// fake ports.
fn enter(harness: &mut Harness) -> Vec<Effect> {
    let before = harness.effects.len();
    harness.settle(Event::Key(Key::Enter));
    harness.effects[before..].to_vec()
}

#[test]
fn browsing_refreshing_selecting_and_opening_every_sheet_write_nothing() {
    let mut harness = started(workflow(false));
    queue_round(&harness);
    harness.settle(Event::Key(Key::Char('r')));
    let references: Vec<String> = harness
        .view
        .list
        .rows
        .iter()
        .map(|r| r.reference.clone())
        .collect();
    for reference in ["TEAM-8", "TEAM-6", "TEAM-7", "TEAM-3", "#9"] {
        assert!(references.iter().any(|r| r == reference), "{references:?}");
    }
    for reference in &references {
        open(&mut harness, reference);
        // Keys on the sheet that aren't `Enter` write nothing either.
        for key in [Key::Char('m'), Key::Char('t'), Key::Char('u'), Key::Down] {
            harness.settle(Event::Key(key));
        }
        harness.settle(Event::Key(Key::Esc));
        assert!(harness.view.overlay.is_none(), "{reference}'s sheet closed");
    }
    harness.settle(Event::Key(Key::Down));
    harness.settle(Event::Key(Key::Up));
    assert!(
        !harness.effects.iter().any(is_linear),
        "{:#?}",
        harness
            .effects
            .iter()
            .filter(|e| is_linear(e))
            .collect::<Vec<_>>()
    );
    assert!(harness.linear_writer.calls().is_empty());
    assert!(harness.linear.reread_requests().is_empty());
}

#[test]
fn start_shows_its_two_writes_then_rereads_assigns_moves_and_launches() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-8");
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    assert_eq!(ready.reference, "TEAM-8");
    assert_eq!(ready.next_step, "Start");
    assert_eq!(
        ready.linear_writes,
        [
            format!("Linear: assign TEAM-8 to {LINEAR_ME_NAME} (you)"),
            format!("Linear: move TEAM-8 from Todo to {IN_PROGRESS}"),
        ]
    );
    assert_eq!(field(&ready, "IN LINEAR").0, "unassigned, in Todo");
    assert_eq!(field(&ready, "MOVE").0, format!("on: to {IN_PROGRESS}"));
    assert_eq!(field(&ready, "TARGET").0, "acme/api");
    assert_eq!(
        field(&ready, "BRANCH"),
        (
            TEAM_8_BRANCH,
            Some("new, from origin/main; tracks nothing until pushed; Linear's suggested name")
        )
    );
    assert!(
        messages(&ready).contains("no compare-and-set"),
        "{}",
        messages(&ready)
    );

    harness.linear.push_reread(reread(None, todo()));
    harness.linear_writer.push_outcome(done(Some(me()), todo()));
    harness
        .linear_writer
        .push_outcome(done(Some(me()), in_progress()));
    let effects = enter(&mut harness);

    assert_eq!(acts(&effects), ["re-read", "assign", "move", "launch"]);
    assert_eq!(
        harness.linear_writer.calls(),
        [
            LinearWriteCall::AssignToViewer {
                issue: "id-8".to_owned()
            },
            // The state named by the re-read's ID for the issue's own team.
            LinearWriteCall::MoveState {
                issue: "id-8".to_owned(),
                state: in_progress(),
            },
        ]
    );
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].next_step, NextStep::Start);
    assert_eq!(plans[0].target.repo, RepoId::new("acme", "api"));
    assert_eq!(
        plans[0].base.reference,
        BaseRef::DefaultBranch("main".to_owned())
    );
    assert_eq!(plans[0].branch.name, TEAM_8_BRANCH);
    assert_eq!(plans[0].branch.start, BranchStart::Fresh);
    assert_eq!(plans[0].session, "api-team-8-start");
    let jobs = harness.jobs.records();
    assert_eq!(jobs.len(), 1);
    assert!(matches!(
        &jobs[0].subject,
        JobSubject::LinearIssue { owner, issue } if owner == "acme" && issue.id == "id-8"
    ));

    let finished = sheet(&harness).clone();
    assert_eq!(
        finished.status,
        LaunchStatus::Launched,
        "{}",
        messages(&finished)
    );
    // The Job is the Linear Issue's: its row shows the Live Job marker.
    harness.settle(Event::Key(Key::Esc));
    let row = harness
        .view
        .list
        .rows
        .iter()
        .find(|r| r.reference == "TEAM-8")
        .expect("TEAM-8 listed");
    assert!(row.job.is_some(), "{row:?}");
    let text = messages(&finished);
    assert!(
        text.contains(&format!(
            "assigned TEAM-8: Linear returned it assigned to {LINEAR_ME_NAME}, in Todo"
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "moved TEAM-8: Linear returned it assigned to {LINEAR_ME_NAME}, in {IN_PROGRESS}"
        )),
        "{text}"
    );
}

#[test]
fn a_reread_showing_a_new_assignee_asks_again_and_writes_nothing() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-8");
    let someone = LinearUser {
        id: "user-other".to_owned(),
        name: "Someone Else".to_owned(),
    };
    harness.linear.push_reread(reread(Some(someone), todo()));
    let effects = enter(&mut harness);

    assert_eq!(acts(&effects), ["re-read"]);
    assert!(harness.linear_writer.calls().is_empty());
    assert!(launch_plans(&harness).is_empty());
    let asked = sheet(&harness);
    assert_eq!(asked.status, LaunchStatus::Ready);
    assert_eq!(
        field(asked, "IN LINEAR").0,
        "assigned to Someone Else, in Todo"
    );
    assert!(
        messages(asked).contains("TEAM-8 is now assigned to Someone Else"),
        "{}",
        messages(asked)
    );

    // Asked again: `Enter` now confirms what the sheet shows.
    harness.linear.push_reread(reread(
        Some(LinearUser {
            id: "user-other".to_owned(),
            name: "Someone Else".to_owned(),
        }),
        todo(),
    ));
    harness.linear_writer.push_outcome(done(Some(me()), todo()));
    harness
        .linear_writer
        .push_outcome(done(Some(me()), in_progress()));
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["re-read", "assign", "move", "launch"]);
}

#[test]
fn a_reread_showing_a_new_state_on_mark_for_release_asks_again_and_writes_nothing() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-3");
    let ready = sheet(&harness).clone();
    assert_eq!(ready.next_step, "Mark for release");
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    assert_eq!(
        ready.linear_writes,
        [format!(
            "Linear: move TEAM-3 from {IN_PROGRESS} to {RELEASE}"
        )]
    );
    assert_eq!(field(&ready, "RELEASE").0, RELEASE);
    let text = messages(&ready);
    assert!(text.contains("evidence: merged acme/api#1"), "{text}");
    assert!(text.contains("den can't see deployments"), "{text}");
    assert!(ready.prompt.is_none() && ready.command.is_none());

    harness.linear.push_reread(reread(Some(me()), release()));
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["re-read"]);
    assert!(harness.linear_writer.calls().is_empty());
    let asked = sheet(&harness);
    assert_eq!(asked.status, LaunchStatus::Ready);
    assert!(
        messages(asked).contains(&format!("TEAM-3 is now in {RELEASE} (was {IN_PROGRESS})")),
        "{}",
        messages(asked)
    );
}

#[test]
fn mark_for_release_rereads_moves_and_shows_what_linear_returned() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-3");
    harness
        .linear
        .push_reread(reread(Some(me()), in_progress()));
    harness
        .linear_writer
        .push_outcome(done(Some(me()), release()));
    let effects = enter(&mut harness);

    assert_eq!(acts(&effects), ["re-read", "move"]);
    assert_eq!(
        harness.linear_writer.calls(),
        [LinearWriteCall::MoveState {
            issue: "id-3".to_owned(),
            state: release(),
        }]
    );
    let moved = sheet(&harness);
    assert_eq!(moved.status, LaunchStatus::Launched);
    assert!(
        messages(moved).contains(&format!(
            "moved TEAM-3: Linear returned it assigned to {LINEAR_ME_NAME}, in {RELEASE}"
        )),
        "{}",
        messages(moved)
    );
    assert!(
        harness.jobs.records().is_empty(),
        "no Job for Mark for release"
    );
}

#[test]
fn assign_done_then_move_failed_stops_reports_and_neither_launches_nor_unassigns() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-8");
    harness.linear.push_reread(reread(None, todo()));
    harness.linear_writer.push_outcome(done(Some(me()), todo()));
    harness
        .linear_writer
        .push_outcome(LinearWriteOutcome::Failed {
            error: "state not allowed".to_owned(),
        });
    let effects = enter(&mut harness);

    assert_eq!(acts(&effects), ["re-read", "assign", "move"]);
    // Exactly the two confirmed writes: nothing un-assigns or moves back.
    assert_eq!(harness.linear_writer.calls().len(), 2);
    assert!(launch_plans(&harness).is_empty());
    assert!(harness.jobs.records().is_empty());
    assert!(harness.workspace.calls().is_empty());
    let stopped = sheet(&harness);
    assert_eq!(stopped.status, LaunchStatus::Failed);
    assert_eq!(stopped.headline, "Stopped: nothing launched");
    let text = messages(stopped);
    assert!(text.contains("assigned TEAM-8"), "{text}");
    assert!(
        text.contains(&format!(
            "moving TEAM-8 to {IN_PROGRESS} failed: state not allowed"
        )),
        "{text}"
    );
    assert!(text.contains("never un-assigns"), "{text}");

    // The stopped sheet only closes.
    let effects = enter(&mut harness);
    assert!(acts(&effects).is_empty());
    assert!(harness.view.overlay.is_none());
}

/// The writes were made, then re-verification refused the launch (the Main
/// Clone lost its remote): the sheet never says nothing was done.
#[test]
fn a_launch_refused_after_the_writes_says_the_writes_stand() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-8");
    harness.linear.push_reread(reread(None, todo()));
    harness.linear_writer.push_outcome(done(Some(me()), todo()));
    harness
        .linear_writer
        .push_outcome(done(Some(me()), in_progress()));
    harness.workspace.add_clone(API_CLONE, &[]);
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["re-read", "assign", "move", "launch"]);
    let refused = sheet(&harness);
    assert_eq!(
        refused.status,
        LaunchStatus::Refused,
        "{}",
        messages(refused)
    );
    assert!(
        refused.headline.contains("nothing launched")
            && refused.headline.contains("Linear writes above were made"),
        "{}",
        refused.headline
    );
    assert!(!refused.headline.contains("nothing was done"));
    assert!(messages(refused).contains("assigned TEAM-8"));
}

#[test]
fn an_unknown_write_outcome_is_followed_by_a_reread_and_never_retried() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-8");
    harness.linear.push_reread(reread(None, todo()));
    harness
        .linear_writer
        .push_outcome(LinearWriteOutcome::Unknown {
            why: "Linear did not answer within 20s".to_owned(),
        });
    harness.linear.push_reread(reread(Some(me()), todo()));
    let effects = enter(&mut harness);

    // The very next request after the unknown write is a re-read.
    assert_eq!(acts(&effects), ["re-read", "assign", "re-read"]);
    assert_eq!(harness.linear_writer.calls().len(), 1, "no retry");
    assert_eq!(harness.linear.reread_requests().len(), 2);
    assert!(launch_plans(&harness).is_empty());
    let stopped = sheet(&harness);
    assert_eq!(stopped.status, LaunchStatus::Failed);
    let text = messages(stopped);
    assert!(text.contains("assigning TEAM-8: unknown"), "{text}");
    assert!(
        text.contains(&format!(
            "Linear now shows TEAM-8 assigned to {LINEAR_ME_NAME}, in Todo"
        )),
        "{text}"
    );
}

#[test]
fn an_unknown_move_is_followed_by_a_reread_and_never_retried() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-3");
    harness
        .linear
        .push_reread(reread(Some(me()), in_progress()));
    harness
        .linear_writer
        .push_outcome(LinearWriteOutcome::Unknown {
            why: "Linear answered HTTP 502".to_owned(),
        });
    harness.linear.push_reread(reread(Some(me()), release()));
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["re-read", "move", "re-read"]);
    assert_eq!(harness.linear_writer.calls().len(), 1);
    let stopped = sheet(&harness);
    assert_eq!(stopped.status, LaunchStatus::Failed);
    assert!(
        messages(stopped).contains(&format!(
            "Linear now shows TEAM-3 assigned to {LINEAR_ME_NAME}, in {RELEASE}"
        )),
        "{}",
        messages(stopped)
    );
}

#[test]
fn a_failed_reread_writes_nothing() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-8");
    harness.linear.push_reread(LinearIssueRead::Failed {
        error: "could not reach Linear".to_owned(),
    });
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["re-read"]);
    assert!(harness.linear_writer.calls().is_empty());
    assert!(launch_plans(&harness).is_empty());
    assert!(
        messages(sheet(&harness)).contains("so it wrote nothing: could not reach Linear"),
        "{}",
        messages(sheet(&harness))
    );
}

#[test]
fn the_implement_prompt_carries_the_identifier_and_url_and_no_issue_text() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-6");
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    assert_eq!(ready.next_step, "Implement");
    // Implement claims nothing; the issue hasn't started, so the move is offered.
    assert_eq!(
        ready.linear_writes,
        [format!("Linear: move TEAM-6 from Todo to {IN_PROGRESS}")]
    );
    let url = team_6().url;
    let prompt = ready.prompt.expect("a prompt").text;
    assert_eq!(prompt, format!("/implement TEAM-6 {url} in acme/api"));
    assert!(!prompt.contains("SECRET"), "{prompt}");

    harness.linear.push_reread(reread(Some(me()), todo()));
    harness
        .linear_writer
        .push_outcome(done(Some(me()), in_progress()));
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["re-read", "move", "launch"]);
    let plan = &launch_plans(&harness)[0];
    assert!(plan.prompt.contains("TEAM-6") && plan.prompt.contains(&url));
    assert!(!plan.prompt.contains("SECRET"));
    // No safe name from Linear: den's own `implement_` branch.
    assert_eq!(plan.branch.name, "implement_team-6-secret-title-text");
}

#[test]
fn implement_on_a_started_issue_writes_nothing_and_launches_directly() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-7");
    let ready = sheet(&harness).clone();
    assert!(ready.linear_writes.is_empty(), "{:?}", ready.linear_writes);
    assert_eq!(field(&ready, "MOVE").0, "none");
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["launch"]);
    assert!(harness.linear.reread_requests().is_empty());
    assert!(harness.linear_writer.calls().is_empty());
}

#[test]
fn the_move_toggle_turns_the_move_off_and_start_still_claims() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-8");
    harness.settle(Event::Key(Key::Char('m')));
    let off = sheet(&harness).clone();
    assert_eq!(field(&off, "MOVE").0, "off");
    assert_eq!(
        off.linear_writes,
        [format!("Linear: assign TEAM-8 to {LINEAR_ME_NAME} (you)")]
    );
    harness.linear.push_reread(reread(None, todo()));
    harness.linear_writer.push_outcome(done(Some(me()), todo()));
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["re-read", "assign", "launch"]);
}

#[test]
fn several_repos_for_new_work_wait_for_one_to_be_chosen() {
    let mut harness = started(workflow(true));
    open(&mut harness, "TEAM-8");
    let choosing = sheet(&harness).clone();
    assert_eq!(choosing.status, LaunchStatus::ChooseRepo);
    assert_eq!(field(&choosing, "REPO").0, "none chosen");
    assert!(
        !harness
            .effects
            .iter()
            .any(|e| matches!(e, Effect::InspectLaunch { .. }))
    );
    // Enter doesn't guess.
    let effects = enter(&mut harness);
    assert!(acts(&effects).is_empty());
    assert!(messages(sheet(&harness)).contains("choose the target Repo with t"));

    harness.settle(Event::Key(Key::Char('t')));
    assert_eq!(field(sheet(&harness), "TARGET").0, "acme/api");
    harness.settle(Event::Key(Key::Char('t')));
    let web = sheet(&harness).clone();
    assert_eq!(web.status, LaunchStatus::Ready, "{}", messages(&web));
    assert_eq!(field(&web, "TARGET").0, "acme/web");
    assert!(
        web.prompt.expect("a prompt").text.ends_with("in acme/web"),
        "the prompt names the chosen Repo"
    );
}

#[test]
fn no_repos_for_new_work_refuses_without_touching_the_workspace() {
    let mut empty = workflow(false);
    empty.new_work_repos.clear();
    let mut harness = started(empty);
    open(&mut harness, "TEAM-8");
    let refused = sheet(&harness);
    assert_eq!(refused.status, LaunchStatus::Refused);
    assert!(messages(refused).contains("offers no Repos for new work"));
    assert!(
        !harness
            .effects
            .iter()
            .any(|e| matches!(e, Effect::InspectLaunch { .. }))
    );
}

#[test]
fn no_new_work_repos_keeps_an_existing_linear_job_reopenable() {
    let mut empty = workflow(false);
    empty.new_work_repos.clear();
    let mut harness = started(empty);
    let worktree = format!("{API_CLONE}/.claude/worktrees/implement-team-7");
    let job = JobRecord {
        subject: JobSubject::of(&team_7().work_item_id()),
        next_step: NextStep::Implement,
        target: RepoId::new("acme", "api"),
        main_clone: API_CLONE.into(),
        worktree: worktree.clone().into(),
        branch: "implement-team-7".to_owned(),
        session: "api-team-7-implement".to_owned(),
        launched_at: support::build::now(),
        permission_mode: None,
    };
    harness.jobs.holding(job.clone());
    harness.workspace.add_existing(&worktree);
    harness
        .workspace
        .register_worktree(API_CLONE, &worktree, Some(&job.branch));
    harness.perform(&Effect::CheckJobs);

    open(&mut harness, "TEAM-7");
    assert_eq!(sheet(&harness).status, LaunchStatus::LiveJob);
    assert!(messages(sheet(&harness)).contains("offers no Repos for new work"));
    harness.settle(Event::Key(Key::Char('f')));
    assert_eq!(sheet(&harness).status, LaunchStatus::LiveJob);
    harness.settle(Event::Key(Key::Char('o')));
    assert_eq!(sheet(&harness).status, LaunchStatus::ReopenReady);
    assert_eq!(field(sheet(&harness), "WORKTREE").0, worktree);
    enter(&mut harness);
    assert_eq!(sheet(&harness).status, LaunchStatus::Reopened);
    assert_eq!(harness.launcher.windows()[0].session, job.session);
    assert_eq!(
        harness.launcher.windows()[0].setting_sources,
        den::handoff::command::SettingSources::All,
    );
    assert_eq!(harness.jobs.records(), [job]);
    assert!(launch_plans(&harness).is_empty());
    assert!(harness.workspace.calls().is_empty());
    assert!(!harness.effects.iter().any(is_linear));
}

#[test]
fn an_unsafe_linear_branch_name_is_replaced_by_dens_own() {
    let mut harness = Harness::new(Ok(config(workflow(false))));
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .fully_read_owner("octo")
        .build()));
    let mut answer = linear_answer();
    answer.part.issues = vec![
        linear_issue("acme", "id-8", "TEAM-8")
            .up_for_grabs()
            .title("Pick me")
            .branch_name("-x/../HEAD.lock")
            .build(),
    ];
    harness.linear.push_answer(answer);
    harness
        .workspace
        .add_clone(API_CLONE, &[("origin", "git@github.com:acme/api.git")]);
    harness
        .workspace
        .set_remote_head(API_CLONE, "origin", "main", MAIN_TIP);
    harness.start();
    open(&mut harness, "TEAM-8");
    let ready = sheet(&harness);
    let (branch, note) = field(ready, "BRANCH");
    assert_eq!(branch, "implement_team-8-pick-me");
    assert!(
        note.unwrap().contains("isn't a safe branch name"),
        "{note:?}"
    );
}

#[test]
fn a_github_owners_launch_sheet_shows_no_linear_writes() {
    let mut harness = started(workflow(false));
    open(&mut harness, "#9");
    let ready = sheet(&harness);
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(ready));
    assert!(ready.linear_writes.is_empty());
    assert!(
        ready
            .fields
            .iter()
            .all(|f| f.label != "IN LINEAR" && f.label != "MOVE")
    );
    let effects = enter(&mut harness);
    assert_eq!(acts(&effects), ["launch"]);
    assert!(!harness.effects.iter().any(is_linear_write));
}

#[test]
fn linear_implement_and_review_templates_coexist() {
    use den::handoff::prompts::{self, PromptFields, PromptKind};
    let prompts = prompts::parse(
        std::path::Path::new("prompts.toml"),
        r#"
implement_linear = "Work on {identifier} in {repo}: {url}"
review_linear = "Review {url}{linear_context}{verdict_only}"
"#,
    )
    .unwrap();
    let review = prompts.render(
        PromptKind::ReviewLinear,
        &PromptFields {
            url: "https://github.com/acme/api/pull/1".to_owned(),
            linear_issues: vec![("TEAM-8".to_owned(), team_8().url)],
            ..PromptFields::default()
        },
    );
    assert!(review.contains("TEAM-8"));
    assert!(review.contains("Return your verdict to me only"));
    let mut harness = started(workflow(false));
    harness.app = harness.app.with_launch_settings(Ok(prompts), None);
    open(&mut harness, "TEAM-8");
    assert_eq!(
        sheet(&harness).prompt.as_ref().unwrap().text,
        format!("Work on TEAM-8 in acme/api: {}", team_8().url)
    );
    assert!(
        prompts::parse(
            std::path::Path::new("prompts.toml"),
            "review_linear = '{identifier}{linear_context}{verdict_only}'"
        )
        .is_err()
    );
}

fn drawn(harness: &mut Harness, (cols, rows): (u16, u16)) -> String {
    use den::ui::{Theme, frame};
    use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};
    harness.send(Event::Resized { cols, rows });
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    frame::buffer_text(&buffer)
}

/// The Start sheet and the Mark for release sheet draw their Linear writes
/// as text (each one line, labelled LINEAR), at the design size and at 80×24.
#[test]
fn the_linear_writes_are_drawn_as_text_on_both_sheets() {
    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-8");
    for size in [(160, 45), (80, 24)] {
        let text = drawn(&mut harness, size);
        assert!(text.contains("LINEAR"), "{text}");
        assert!(text.contains("Linear: assign TEAM-8"), "{text}");
    }
    harness.settle(Event::Key(Key::Esc));
    open(&mut harness, "TEAM-3");
    let text = drawn(&mut harness, (160, 45));
    assert!(text.contains("M A R K   F O R   R E L E A S E"), "{text}");
    assert!(text.contains("Linear: move TEAM-3"), "{text}");
}

#[test]
fn release_evidence_cannot_hide_the_linear_write_preview() {
    use den::ui::{Theme, frame};
    use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};

    let mut harness = started(workflow(false));
    open(&mut harness, "TEAM-3");
    let evidence = (1..=100)
        .map(|n| format!("acme/api#{n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Unicode,
    ))
    .unwrap();
    for (cols, rows) in [(160, 45), (80, 24)] {
        harness.send(Event::Resized { cols, rows });
        let mut view = harness.view.clone();
        let Some(Overlay::LaunchSheet(sheet)) = &mut view.overlay else {
            panic!("release sheet")
        };
        sheet.messages.push(SheetMessage {
            text: format!("evidence: merged {evidence}"),
            tone: Tone::Plain,
        });
        let buffer = frame::render(&theme, &view, cols, rows).unwrap();
        let text = frame::buffer_text(&buffer);
        assert!(text.contains("LINEAR"), "{text}");
        assert!(text.contains("Linear: move TEAM-3"), "{text}");
        assert!(
            text.contains(RELEASE),
            "the move destination stays visible: {text}"
        );
    }
}

#[test]
#[ignore = "prints the sheets for a look by eye"]
fn print_the_linear_sheets() {
    let mut harness = started(workflow(true));
    open(&mut harness, "TEAM-8");
    harness.settle(Event::Key(Key::Char('t')));
    for size in [(160, 45), (80, 24)] {
        println!("{}\n", drawn(&mut harness, size));
    }
    harness.settle(Event::Key(Key::Esc));
    open(&mut harness, "TEAM-3");
    for size in [(160, 45), (80, 24)] {
        println!("{}\n", drawn(&mut harness, size));
    }
}

#[test]
fn release_evidence_cannot_hide_write_failures_or_unknown_outcomes() {
    use den::ui::{Theme, frame};
    use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};

    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Unicode,
    ))
    .unwrap();
    for unknown in [false, true] {
        let mut harness = started(workflow(false));
        open(&mut harness, "TEAM-3");
        harness
            .linear
            .push_reread(reread(Some(me()), in_progress()));
        harness.linear_writer.push_outcome(if unknown {
            LinearWriteOutcome::Unknown {
                why: "HTTP 502".to_owned(),
            }
        } else {
            LinearWriteOutcome::Failed {
                error: "state not allowed".to_owned(),
            }
        });
        if unknown {
            harness.linear.push_reread(reread(Some(me()), release()));
        }
        enter(&mut harness);
        for (cols, rows) in [(160, 45), (80, 24)] {
            harness.send(Event::Resized { cols, rows });
            let mut view = harness.view.clone();
            let Some(Overlay::LaunchSheet(sheet)) = &mut view.overlay else {
                panic!("release sheet")
            };
            // Enlarge the real evidence message without changing its position.
            let evidence = sheet
                .messages
                .iter_mut()
                .find(|m| m.text.starts_with(den::decide::EVIDENCE_PREFIX))
                .expect("release evidence");
            evidence.text.push_str(&" acme/api#123,".repeat(100));
            let buffer = frame::render(&theme, &view, cols, rows).unwrap();
            let text = frame::buffer_text(&buffer);
            // At 80 columns, complete error sentences can wrap across frame rows.
            let visible = text
                .lines()
                .map(|line| line.split('┊').nth(1).unwrap_or(line))
                .flat_map(str::split_whitespace)
                .collect::<Vec<_>>()
                .join(" ");
            if unknown {
                assert!(visible.contains("unknown (HTTP 502)"), "{text}");
                assert!(
                    visible.contains(&format!(
                        "Linear now shows TEAM-3 assigned to {LINEAR_ME_NAME}, in {RELEASE}"
                    )),
                    "{text}"
                );
            } else {
                assert!(visible.contains("state not allowed"), "{text}");
            }
        }
    }
}

/// Round 4 of #52: an unassigned issue outside the start teams is still shown
/// when its Landing Group decides (here, an open member). It isn't offered to
/// Start, so it never gets Start, and `c` on it neither plans a Job nor
/// emits any Linear Effect.
#[test]
fn an_issue_shown_only_for_its_group_is_never_started() {
    let mut harness = Harness::new(Ok(config(workflow(false))));
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .pull_request(support::build::pr("acme/api", 2).author(ME))
        .fully_read_owner("octo")
        .build()));
    let mut answer = linear_answer();
    answer.part.issues = vec![
        linear_issue("acme", "id-5", "OPS-5")
            .up_for_grabs()
            .attached("acme/api", 2)
            .build(),
    ];
    harness.linear.push_answer(answer);
    harness
        .workspace
        .add_clone(API_CLONE, &[("origin", "git@github.com:acme/api.git")]);
    harness.start();
    let row = harness
        .view
        .list
        .rows
        .iter()
        .find(|r| r.reference == "OPS-5")
        .unwrap_or_else(|| panic!("OPS-5 shown for its group: {:?}", harness.rows()))
        .clone();
    assert_ne!(row.next_step, Some(NextStep::Start), "{row:?}");

    open(&mut harness, "OPS-5");
    let refused = sheet(&harness);
    assert_eq!(
        refused.status,
        LaunchStatus::Refused,
        "{}",
        messages(refused)
    );
    assert!(refused.linear_writes.is_empty());
    assert!(
        !harness
            .effects
            .iter()
            .any(|e| is_linear(e) || matches!(e, Effect::InspectLaunch { .. }))
    );
    let effects = enter(&mut harness);
    assert!(acts(&effects).is_empty());
    assert!(harness.linear_writer.calls().is_empty());
}
