//! Seam 2: launching a Job for a PR (#11). Keys go in, and the tests assert on
//! the ViewModel's launch sheet, the LaunchPlans in Effects, and what the fake
//! Workspace, Launcher and Job store were asked to do.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Key,
        view_model::{LaunchSheet, LaunchStatus, Overlay},
    },
    config::Config,
    domain::Observation,
    handoff::{
        BranchStart, LaunchPlan, WindowCommand,
        prompts::{self, Prompts},
    },
    jobs::{JobRecord, JobStoreError},
    ports::{WorktreeAdd, WorktreeStart},
};
use std::{collections::BTreeMap, path::PathBuf};
use support::{
    build::{ME, ObservationBuilder, now, pr},
    fakes::WorkspaceCall,
    harness::Harness,
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/app";
const WORKTREE: &str = "/home/me/dev/app/.claude/worktrees/feature-7";
/// The default head commit of PR 7 (`pr()` uses the number in hex).
const HEAD_7: &str = "0000000000000000000000000000000000000007";
const MOVED_HEAD: &str = "1111111111111111111111111111111111111111";

fn launch_config(main_clones: &[(&str, &str)]) -> Config {
    Config {
        owners: vec!["octo".to_owned(), "other".to_owned()],
        main_clone_root: Some(PathBuf::from(ROOT)),
        main_clones: main_clones
            .iter()
            .map(|(repo, path)| ((*repo).to_owned(), PathBuf::from(path)))
            .collect::<BTreeMap<_, _>>(),
    }
}

/// octo/app#7, Waiting On me to Address review (changes requested).
fn observation() -> ObservationBuilder {
    ObservationBuilder::new()
        .owner("octo")
        .owner("other")
        .repo("octo/app")
        .pull_request(pr("octo/app", 7).author(ME).changes_requested())
}

fn started(config: Config, observation: Observation) -> Harness {
    let mut harness = Harness::new(Ok(config));
    harness.answer(Ok(observation));
    harness.start();
    harness
}

/// A Main Clone of octo/app whose `upstream` (not `origin`) points at it.
fn with_upstream_clone(harness: &Harness) {
    harness.workspace.add_clone(
        CLONE,
        &[
            ("origin", "https://github.com/me/app-fork.git"),
            ("upstream", "git@github.com:octo/app.git"),
        ],
    );
    harness
        .workspace
        .set_remote_head(CLONE, "upstream", "feature-7", HEAD_7);
}

fn sheet(harness: &Harness) -> &LaunchSheet {
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => sheet,
        other => panic!("no launch sheet: {other:?}"),
    }
}

fn press(harness: &mut Harness, key: Key) {
    harness.settle(den::app::Event::Key(key));
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

#[test]
fn c_then_enter_on_a_pr_produces_exactly_one_launch_plan_fetching_from_the_remote_whose_url_is_the_prs_repo()
 {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);

    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    assert_eq!(ready.reference, "octo/app#7");
    assert_eq!(ready.next_step, "Address review");
    assert_eq!(field(&ready, "TARGET").0, "octo/app");
    assert_eq!(
        field(&ready, "MAIN CLONE"),
        (
            CLONE,
            Some("exists, writable; found under /home/me/dev by its remote")
        )
    );
    assert_eq!(
        field(&ready, "REMOTE").0,
        "upstream → git@github.com:octo/app.git"
    );
    assert_eq!(field(&ready, "BASE").0, "PR head feature-7 @ 0000000");
    assert_eq!(field(&ready, "WORKTREE"), (WORKTREE, Some("free")));
    let prompt = ready.prompt.as_ref().expect("a prompt");
    assert_eq!(
        prompt.text,
        "/gh-resolve https://github.com/octo/app/pull/7"
    );
    assert_eq!(
        ready.command.as_deref(),
        Some(
            "kitty --detach --hold --directory /home/me/dev/app/.claude/worktrees/feature-7 claude -n app-pr7-address-review '/gh-resolve https://github.com/octo/app/pull/7'"
        )
    );

    press(&mut harness, Key::Enter);

    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1, "{plans:?}");
    let plan = &plans[0];
    assert_eq!(plan.target.repo.to_string(), "octo/app");
    assert_eq!(plan.remote.name, "upstream");
    assert_eq!(plan.base.commit, HEAD_7);
    assert_eq!(
        harness.workspace.calls(),
        [
            WorkspaceCall::Fetch {
                clone: CLONE.into(),
                remote: "upstream".to_owned(),
                refspec: "+refs/heads/feature-7:refs/remotes/upstream/feature-7".to_owned(),
            },
            WorkspaceCall::AddWorktree {
                clone: CLONE.into(),
                add: WorktreeAdd {
                    path: WORKTREE.into(),
                    branch: "feature-7".to_owned(),
                    start: WorktreeStart::NewBranchTracking {
                        start: "refs/remotes/upstream/feature-7".to_owned()
                    },
                },
            },
        ]
    );
    assert_eq!(
        harness.launcher.windows(),
        [WindowCommand {
            directory: WORKTREE.into(),
            session: "app-pr7-address-review".to_owned(),
            prompt: "/gh-resolve https://github.com/octo/app/pull/7".to_owned(),
        }]
    );
    let job = harness.jobs.records();
    assert_eq!(job.len(), 1);
    assert_eq!(job[0].worktree, PathBuf::from(WORKTREE));
    assert_eq!(job[0].main_clone, PathBuf::from(CLONE));
    assert_eq!(job[0].branch, "feature-7");
    assert_eq!(job[0].session, "app-pr7-address-review");
    assert_eq!(job[0].target.to_string(), "octo/app");
    assert_eq!(job[0].launched_at, now());

    let done = sheet(&harness);
    assert_eq!(done.status, LaunchStatus::Launched, "{}", messages(done));
    assert!(
        messages(done).contains("opened a kitty window"),
        "{}",
        messages(done)
    );
}

#[test]
fn a_fork_pr_targets_the_fork_and_fetches_from_the_remote_pointing_at_the_fork() {
    let mut harness = started(
        launch_config(&[]),
        ObservationBuilder::new()
            .owner("octo")
            .fork("octo/app", Some("upstream-org/app"))
            .pull_request(pr("octo/app", 7).author(ME).changes_requested())
            .build(),
    );
    harness.workspace.add_clone(
        CLONE,
        &[
            ("parent", "https://github.com/upstream-org/app.git"),
            ("mine", "https://github.com/octo/app"),
        ],
    );
    harness
        .workspace
        .set_remote_head(CLONE, "mine", "feature-7", HEAD_7);

    press(&mut harness, Key::Char('c'));
    let (target, note) = field(sheet(&harness), "TARGET");
    assert_eq!(target, "octo/app");
    assert_eq!(note, Some("fork: its own Repo, not its Upstream"));
    press(&mut harness, Key::Enter);

    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].target.repo.to_string(), "octo/app");
    assert_eq!(plans[0].remote.name, "mine");
    assert!(matches!(
        &harness.workspace.calls()[0],
        WorkspaceCall::Fetch { remote, .. } if remote == "mine"
    ));
}

#[test]
fn the_same_pr_number_in_two_repos_where_the_main_clone_points_at_the_other_repo_is_refused() {
    let observation = observation()
        .repo("other/app")
        .pull_request(pr("other/app", 7).author(ME).changes_requested())
        .build();
    let mut harness = started(launch_config(&[]), observation);
    // The only clone named `app` is other/app's, and the folder den would
    // clone octo/app into instead (#12) is taken too.
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "git@github.com:other/app.git")]);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-7", HEAD_7);
    harness
        .workspace
        .add_existing("/home/me/dev/octo-app/notes.txt");
    let octo_row = harness
        .view
        .list
        .rows
        .iter()
        .position(|r| r.repo == "octo/app")
        .expect("octo/app#7 is listed");
    for _ in 0..octo_row {
        press(&mut harness, Key::Down);
    }

    press(&mut harness, Key::Char('c'));
    let refused = sheet(&harness).clone();
    assert_eq!(refused.reference, "octo/app#7");
    assert_eq!(refused.status, LaunchStatus::Refused);
    let text = messages(&refused);
    assert!(
        text.contains("no clone under /home/me/dev has a remote for octo/app"),
        "{text}"
    );
    assert!(
        text.contains("/home/me/dev/app is named like it but its remotes are origin → git@github.com:other/app.git"),
        "{text}"
    );
    assert!(
        text.contains("den won't clone octo/app into a folder that exists: /home/me/dev/app and /home/me/dev/octo-app both exist"),
        "{text}"
    );
    press(&mut harness, Key::Enter);
    assert!(launch_plans(&harness).is_empty());
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn a_configured_main_clone_without_a_remote_for_the_target_is_refused_not_replaced() {
    let mut harness = started(launch_config(&[("octo/app", CLONE)]), observation().build());
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "git@github.com:other/app.git")]);
    // A clone that would match is ignored: the config says which one.
    harness.workspace.add_clone(
        "/home/me/dev/app-2",
        &[("origin", "https://github.com/octo/app")],
    );

    press(&mut harness, Key::Char('c'));
    let refused = sheet(&harness);
    assert_eq!(refused.status, LaunchStatus::Refused);
    assert_eq!(
        messages(refused),
        "the Main Clone /home/me/dev/app has no remote for octo/app; its remotes are origin → git@github.com:other/app.git"
    );
}

#[test]
fn a_pr_head_that_moved_between_sheet_and_launch_asks_again_then_launches_on_the_new_head() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    press(&mut harness, Key::Char('c'));
    assert_eq!(sheet(&harness).status, LaunchStatus::Ready);

    harness
        .workspace
        .set_remote_head(CLONE, "upstream", "feature-7", MOVED_HEAD);
    press(&mut harness, Key::Enter);

    let asked = sheet(&harness).clone();
    assert_eq!(asked.status, LaunchStatus::HeadMoved);
    assert_eq!(
        asked.headline,
        "The PR head moved from 0000000 to 1111111: check, then ⏎ launches on 1111111"
    );
    assert_eq!(
        field(&asked, "BASE"),
        ("PR head feature-7 @ 1111111", Some("moved from 0000000"))
    );
    assert!(harness.workspace.calls().is_empty(), "nothing was done");
    assert!(harness.launcher.windows().is_empty());

    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(
        plans.len(),
        2,
        "one refused by the moved head, one launched"
    );
    assert_eq!(plans[1].base.commit, MOVED_HEAD);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
    assert_eq!(harness.launcher.windows().len(), 1);
}

#[test]
fn an_archived_repo_refuses_without_looking_at_the_workspace() {
    let mut harness = started(
        launch_config(&[]),
        ObservationBuilder::new()
            .owner("octo")
            .archived_repo("octo/app")
            .pull_request(pr("octo/app", 7).author(ME).changes_requested())
            .build(),
    );
    with_upstream_clone(&harness);
    press(&mut harness, Key::Char('c'));
    let refused = sheet(&harness);
    assert_eq!(refused.status, LaunchStatus::Refused);
    assert_eq!(
        messages(refused),
        "octo/app is archived, so work there can't be pushed"
    );
    assert!(
        !harness
            .effects
            .iter()
            .any(|e| matches!(e, Effect::InspectLaunch { .. }))
    );
}

#[test]
fn outside_kitty_the_command_is_copied_to_the_clipboard_and_shown() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness.launcher.outside_kitty();

    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    let command = "cd /home/me/dev/app/.claude/worktrees/feature-7 && claude -n app-pr7-address-review '/gh-resolve https://github.com/octo/app/pull/7'";
    assert_eq!(ready.command.as_deref(), Some(command));
    assert_eq!(
        field(&ready, "OPENS"),
        (
            "nothing: the command is copied to the clipboard",
            Some("den is not running in kitty")
        )
    );
    press(&mut harness, Key::Enter);

    assert_eq!(harness.launcher.clipboard(), [command]);
    assert!(harness.launcher.windows().is_empty());
    let done = sheet(&harness);
    assert_eq!(done.status, LaunchStatus::Launched);
    assert_eq!(done.command.as_deref(), Some(command));
    assert!(
        messages(done).contains("The command is on the clipboard"),
        "{}",
        messages(done)
    );
    assert_eq!(harness.jobs.records().len(), 1);
}

#[test]
fn the_prompt_is_edited_in_place_and_the_edit_is_what_launches() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    press(&mut harness, Key::Char('c'));

    press(&mut harness, Key::Char('e'));
    assert!(sheet(&harness).prompt.as_ref().expect("prompt").editing);
    // While editing, `q`, `c` and `u` are text, not commands.
    for key in [
        Key::End,
        Key::Char(' '),
        Key::Char('q'),
        Key::Char('u'),
        Key::Char('x'),
    ] {
        press(&mut harness, key);
    }
    press(&mut harness, Key::Backspace); // drop the x
    press(&mut harness, Key::Home);
    press(&mut harness, Key::Delete); // drop the leading /
    press(&mut harness, Key::Char('/'));
    press(&mut harness, Key::Esc); // stops editing, keeps the sheet open
    let edited = sheet(&harness).clone();
    let prompt = edited.prompt.as_ref().expect("prompt");
    assert!(!prompt.editing);
    assert_eq!(
        prompt.text,
        "/gh-resolve https://github.com/octo/app/pull/7 qu"
    );
    assert!(
        edited
            .command
            .as_deref()
            .is_some_and(|c| c.ends_with("'/gh-resolve https://github.com/octo/app/pull/7 qu'")),
        "{:?}",
        edited.command
    );

    press(&mut harness, Key::Enter);
    assert_eq!(
        harness.launcher.windows()[0].prompt,
        "/gh-resolve https://github.com/octo/app/pull/7 qu"
    );
}

#[test]
fn an_empty_or_option_like_prompt_is_not_launched() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Char('e'));
    press(&mut harness, Key::Home);
    for _ in 0..80 {
        press(&mut harness, Key::Delete);
    }
    press(&mut harness, Key::Enter); // stop editing
    press(&mut harness, Key::Enter); // try to launch
    let blocked = sheet(&harness).clone();
    assert_eq!(blocked.status, LaunchStatus::Ready);
    assert_eq!(messages(&blocked), "the prompt is empty; edit it with e");

    for key in [
        Key::Char('e'),
        Key::Char('-'),
        Key::Char('x'),
        Key::Esc,
        Key::Enter,
    ] {
        press(&mut harness, key);
    }
    assert!(messages(sheet(&harness)).contains("starts with '-'"));
    assert!(launch_plans(&harness).is_empty());
}

#[test]
fn esc_cancels_and_nothing_is_done() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Esc);
    assert_eq!(harness.view.overlay, None);
    assert!(launch_plans(&harness).is_empty());
    assert!(harness.workspace.calls().is_empty());
    assert!(harness.jobs.records().is_empty());
}

#[test]
fn a_failing_step_stops_the_launch_and_reports_what_was_done_without_deleting_it() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness.launcher.fail_window("kitty: cannot open display");
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Enter);

    let failed = sheet(&harness).clone();
    assert_eq!(failed.status, LaunchStatus::Failed);
    assert_eq!(failed.headline, "Stopped: could not open the kitty window");
    let text = messages(&failed);
    for expected in [
        "re-verified: upstream → octo/app at 0000000; the worktree path is free",
        "fetched refs/remotes/upstream/feature-7 from upstream (0000000)",
        "added the worktree /home/me/dev/app/.claude/worktrees/feature-7 on branch feature-7",
        "could not open the kitty window: open a kitty window: kitty: cannot open display",
        "Nothing was deleted; the steps above were left as they are.",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    assert!(harness.jobs.records().is_empty(), "no Job is recorded");
    // The worktree stays: only the two changing calls, no removal.
    assert_eq!(harness.workspace.calls().len(), 2);
}

#[test]
fn a_failing_fetch_names_the_step_and_adds_no_worktree() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness
        .workspace
        .fail("fetch", "git fetch timed out after 120s");
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Enter);
    let failed = sheet(&harness);
    assert_eq!(failed.headline, "Stopped: could not fetch");
    assert!(messages(failed).contains("git fetch timed out after 120s"));
    assert!(
        !harness
            .workspace
            .calls()
            .iter()
            .any(|c| matches!(c, WorkspaceCall::AddWorktree { .. }))
    );
}

#[test]
fn a_main_clone_whose_remote_changed_after_the_sheet_opened_refuses_at_launch_doing_nothing() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    press(&mut harness, Key::Char('c'));
    // Someone repoints `upstream` before Enter.
    harness
        .workspace
        .add_clone(CLONE, &[("upstream", "git@github.com:other/app.git")]);
    press(&mut harness, Key::Enter);
    let refused = sheet(&harness);
    assert_eq!(refused.status, LaunchStatus::Refused);
    assert!(
        messages(refused).contains("has no remote for octo/app"),
        "{}",
        messages(refused)
    );
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn a_worktree_path_that_exists_is_refused_and_a_recorded_job_there_is_named() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness.workspace.add_existing(WORKTREE);
    let item = harness.view.list.rows[0].id.clone();
    harness.jobs.holding(JobRecord {
        work_item: item,
        next_step: den::domain::NextStep::AddressReview,
        target: den::domain::RepoId::new("octo", "app"),
        main_clone: CLONE.into(),
        worktree: WORKTREE.into(),
        branch: "feature-7".to_owned(),
        session: "app-pr7-address-review".to_owned(),
        launched_at: now(),
    });
    press(&mut harness, Key::Char('c'));
    let refused = sheet(&harness);
    assert_eq!(refused.status, LaunchStatus::Refused);
    assert!(
        messages(refused).contains(
            "a Job for this Work Item (session app-pr7-address-review, launched 2026-11-19 12:00 UTC) already has the worktree"
        ),
        "{}",
        messages(refused)
    );
}

#[test]
fn unreadable_job_records_refuse_rather_than_risk_a_duplicate_job() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness.jobs.fail_load(JobStoreError::Read {
        path: "/state/jobs.json".into(),
        detail: "expected value at line 1".to_owned(),
    });
    press(&mut harness, Key::Char('c'));
    assert!(messages(sheet(&harness)).contains("den can't read its Job records"));
}

#[test]
fn a_local_branch_at_another_commit_is_refused_and_one_at_the_head_is_reused() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness
        .workspace
        .set_local_branch(CLONE, "feature-7", MOVED_HEAD);
    press(&mut harness, Key::Char('c'));
    assert!(
        messages(sheet(&harness)).contains(
            "a local branch feature-7 already exists at 1111111, not at the PR head 0000000; den won't move it"
        )
    );
    press(&mut harness, Key::Esc);

    harness
        .workspace
        .set_local_branch(CLONE, "feature-7", HEAD_7);
    press(&mut harness, Key::Char('c'));
    assert_eq!(
        field(sheet(&harness), "BRANCH").1,
        Some("existing local branch, already at the PR head")
    );
    press(&mut harness, Key::Enter);
    assert_eq!(
        launch_plans(&harness)[0].branch.start,
        BranchStart::Existing
    );
    assert!(harness.workspace.calls().iter().any(|c| matches!(
        c,
        WorkspaceCall::AddWorktree { add, .. } if add.start == WorktreeStart::ExistingBranch
    )));
}

#[test]
fn a_local_branch_checked_out_in_another_worktree_is_refused_before_anything_is_fetched() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness
        .workspace
        .set_local_branch(CLONE, "feature-7", HEAD_7);
    harness.workspace.register_worktree(
        CLONE,
        "/home/me/dev/app/.claude/worktrees/review-7",
        Some("feature-7"),
    );
    press(&mut harness, Key::Char('c'));
    assert_eq!(
        messages(sheet(&harness)),
        "the local branch feature-7 is already checked out in /home/me/dev/app/.claude/worktrees/review-7; git checks a branch out in one worktree at a time"
    );
    press(&mut harness, Key::Enter);
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn a_registered_worktree_whose_folder_is_gone_is_refused_not_pruned() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness
        .workspace
        .register_worktree(CLONE, WORKTREE, Some("other"));
    press(&mut harness, Key::Char('c'));
    assert!(
        messages(sheet(&harness)).contains("den doesn't prune worktrees"),
        "{}",
        messages(sheet(&harness))
    );
}

#[test]
fn the_main_clone_prefers_a_folder_name_without_a_numeric_suffix() {
    let mut harness = started(launch_config(&[]), observation().build());
    for path in ["/home/me/dev/app-2", "/home/me/dev/wrk/app", CLONE] {
        harness
            .workspace
            .add_clone(path, &[("origin", "https://github.com/octo/app.git")]);
    }
    press(&mut harness, Key::Char('c'));
    assert_eq!(
        field(sheet(&harness), "MAIN CLONE"),
        (
            CLONE,
            Some(
                "exists, writable; found under /home/me/dev by its remote; also matching: /home/me/dev/wrk/app, /home/me/dev/app-2"
            )
        )
    );
}

#[test]
fn a_main_clone_den_cannot_write_in_is_refused() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness.workspace.make_unwritable(CLONE);
    press(&mut harness, Key::Char('c'));
    assert_eq!(
        messages(sheet(&harness)),
        "den can't write in the Main Clone /home/me/dev/app"
    );
}

#[test]
fn prs_den_cannot_launch_are_refused_with_the_reason() {
    let mut harness = started(
        launch_config(&[]),
        ObservationBuilder::new()
            .owner("octo")
            .repo("octo/app")
            .pull_request(
                pr("octo/app", 7)
                    .author(ME)
                    .changes_requested()
                    .head_from("someone/app"),
            )
            .pull_request(pr("octo/app", 8).author(ME).pending("CI"))
            .build(),
    );
    with_upstream_clone(&harness);
    let rows: Vec<String> = harness
        .view
        .list
        .rows
        .iter()
        .map(|r| r.reference.clone())
        .collect();
    let mut seen = BTreeMap::new();
    for i in 0..rows.len() {
        if i > 0 {
            press(&mut harness, Key::Down);
        }
        press(&mut harness, Key::Char('c'));
        seen.insert(sheet(&harness).reference.clone(), messages(sheet(&harness)));
        press(&mut harness, Key::Esc);
    }
    assert_eq!(
        seen["octo/app#7"],
        "the PR's head branch lives in another Repo (someone/app); launching for cross-Repo heads isn't built yet"
    );
    assert_eq!(
        seen["octo/app#8"],
        "this Work Item has no Next Step to hand off"
    );
}

#[test]
fn without_claude_code_on_path_a_kitty_launch_is_refused() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    harness.launcher.without_claude();
    press(&mut harness, Key::Char('c'));
    assert_eq!(
        messages(sheet(&harness)),
        "Claude Code (claude) is not on PATH"
    );
}

#[test]
fn the_prompts_file_overrides_a_kinds_default_prompt() {
    let prompts = prompts::parse(
        std::path::Path::new("prompts.toml"),
        "address_review = \"/gh-resolve {url} then summarise for {repo}#{number} on {branch}\"\n",
    );
    let mut harness = Harness::new(Ok(launch_config(&[])));
    harness.app = den::app::App::new(Ok(launch_config(&[]))).with_launch_settings(prompts, None);
    harness.answer(Ok(observation().build()));
    harness.start();
    with_upstream_clone(&harness);
    press(&mut harness, Key::Char('c'));
    assert_eq!(
        sheet(&harness).prompt.as_ref().expect("prompt").text,
        "/gh-resolve https://github.com/octo/app/pull/7 then summarise for octo/app#7 on feature-7"
    );
}

#[test]
fn a_malformed_prompts_file_refuses_with_its_file_and_line() {
    let prompts = prompts::parse(
        std::path::Path::new("/cfg/den/prompts.toml"),
        "review = \"/code-review {url}\"\naddress_review = \"/gh-resolve {body}\"\n",
    );
    let mut harness = Harness::new(Ok(launch_config(&[])));
    harness.app = den::app::App::new(Ok(launch_config(&[]))).with_launch_settings(prompts, None);
    harness.answer(Ok(observation().build()));
    harness.start();
    with_upstream_clone(&harness);
    press(&mut harness, Key::Char('c'));
    let text = messages(sheet(&harness));
    assert!(
        text.starts_with("the prompts file is unusable: /cfg/den/prompts.toml, line 2"),
        "{text}"
    );
    assert!(text.contains("unknown placeholder {body}"), "{text}");
}

#[test]
fn the_default_prompts_call_the_skills_for_pr_kinds_and_never_carry_body_text() {
    let defaults = Prompts::default();
    let fields = prompts::PromptFields {
        repo: "octo/app".to_owned(),
        number: Some(7),
        url: "https://github.com/octo/app/pull/7".to_owned(),
        title: "TITLE-TEXT".to_owned(),
        branch: "feature-7".to_owned(),
    };
    use den::domain::NextStep::*;
    let rendered = |step| defaults.render(step, &fields);
    assert_eq!(
        rendered(AddressReview),
        "/gh-resolve https://github.com/octo/app/pull/7"
    );
    assert_eq!(
        rendered(Review),
        "/code-review https://github.com/octo/app/pull/7"
    );
    assert!(rendered(FixCi).starts_with("/diagnosing-bugs "));
    for step in [FinishDraft, ResolveConflicts, Merge] {
        let prompt = rendered(step);
        assert!(!prompt.starts_with('/'), "{prompt}");
        assert!(
            prompt.contains("octo/app#7") && prompt.contains("https://github.com/octo/app/pull/7")
        );
    }
    for step in den::domain::NextStep::ALL {
        assert!(
            !rendered(step).contains("TITLE-TEXT"),
            "{step:?} uses the title"
        );
    }
}

#[test]
fn the_sheet_shows_its_keys_in_the_footer_and_c_is_offered_on_the_list() {
    let mut harness = started(launch_config(&[]), observation().build());
    with_upstream_clone(&harness);
    let keys = |h: &Harness| -> Vec<String> {
        h.view
            .footer
            .hints
            .iter()
            .map(|k| format!("{} {}", k.key, k.label))
            .collect()
    };
    assert!(keys(&harness).contains(&"c launch".to_owned()));
    press(&mut harness, Key::Char('c'));
    assert_eq!(keys(&harness), ["⏎ launch", "e edit", "esc cancel"]);
    press(&mut harness, Key::Char('e'));
    assert_eq!(
        keys(&harness),
        ["⏎ done", "←→ move", "⌫ delete", "esc done"]
    );
}
