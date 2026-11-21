//! Seam 2: Jobs don't inherit den's Linear keys (#70, ADR-0007). Every Job's
//! command removes each distinct `api_key_env` of the configured Linear
//! Workflows with `env -u`, sorted, before `claude`: on the launch sheet, in
//! the Launcher's kitty invocation, on the clipboard and when reopening a
//! Live Job. Without a Linear Workflow the command is milestone 3's.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Event, Key,
        view_model::{LaunchSheet, LaunchStatus, Overlay},
    },
    config::Config,
    domain::{NextStep, RepoId, WorkItemId},
    handoff::command,
    jobs::{JobRecord, JobSubject},
};
use std::path::PathBuf;
use support::{
    build::{ME, ObservationBuilder, now, pr},
    harness::{Harness, config, linear_workflow},
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/app";
const WORKTREE_7: &str = "/home/me/dev/app/.claude/worktrees/feature-7";
const SESSION_7: &str = "app-pr7-address-review";
const PROMPT_7: &str = "'/gh-resolve https://github.com/octo/app/pull/7'";

/// `octo` without a Linear Workflow (its PR is the one launched), and
/// Workflows for other Owners naming `keys`, in this order.
fn config_with_keys(keys: &[(&str, &str)]) -> Config {
    let mut config = config(&["octo"]);
    config.main_clone_root = Some(PathBuf::from(ROOT));
    config.linear = keys
        .iter()
        .map(|(owner, key)| {
            let mut workflow = linear_workflow(owner, owner);
            workflow.api_key_env = (*key).to_owned();
            workflow
        })
        .collect();
    config
}

/// Workflows naming `Z_LINEAR_KEY`, `A_LINEAR_KEY` and `A_LINEAR_KEY` again:
/// out of order, with a duplicate.
fn two_keys() -> Config {
    config_with_keys(&[
        ("zeta", "Z_LINEAR_KEY"),
        ("acme", "A_LINEAR_KEY"),
        ("globex", "A_LINEAR_KEY"),
    ])
}

fn job_7() -> JobRecord {
    JobRecord {
        subject: JobSubject::GitHub(item_7()),
        next_step: NextStep::AddressReview,
        target: RepoId::new("octo", "app"),
        main_clone: CLONE.into(),
        worktree: WORKTREE_7.into(),
        branch: "feature-7".to_owned(),
        session: SESSION_7.to_owned(),
        launched_at: now(),
        permission_mode: None,
    }
}

fn item_7() -> WorkItemId {
    WorkItemId::PullRequest {
        repo: RepoId::new("octo", "app"),
        number: 7,
    }
}

/// den surveying octo/app's PR 7 (mine, changes requested: Address review),
/// with a Main Clone of octo/app; `arrange` runs before den starts.
fn harness(config: Config, arrange: impl FnOnce(&Harness)) -> Harness {
    let mut harness = Harness::new(Ok(config));
    harness
        .workspace
        .add_clone(CLONE, &[("upstream", "git@github.com:octo/app.git")]);
    harness
        .workspace
        .set_remote_head(CLONE, "upstream", "feature-7", &format!("{:040x}", 7));
    arrange(&harness);
    let observed = ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(pr("octo/app", 7).author(ME).changes_requested());
    harness.answer(Ok(observed.build()));
    harness.start();
    let index = harness
        .view
        .list
        .rows
        .iter()
        .position(|row| row.id == item_7())
        .expect("PR 7 is listed");
    for _ in 0..index {
        press(&mut harness, Key::Down);
    }
    harness
}

/// A Live Job for PR 7: recorded, its worktree there and registered.
fn live_job_7(harness: &Harness) {
    harness.jobs.holding(job_7());
    harness.workspace.add_existing(WORKTREE_7);
    harness
        .workspace
        .register_worktree(CLONE, WORKTREE_7, Some("feature-7"));
    harness.workspace.set_local_branch(
        CLONE,
        "feature-7",
        "2222222222222222222222222222222222222222",
    );
}

fn press(harness: &mut Harness, key: Key) {
    harness.settle(Event::Key(key));
}

fn sheet(harness: &Harness) -> &LaunchSheet {
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => sheet,
        other => panic!("expected the launch sheet, got {other:?}"),
    }
}

fn ready_command(harness: &mut Harness) -> String {
    press(harness, Key::Char('c'));
    let ready = sheet(harness);
    assert_eq!(ready.status, LaunchStatus::Ready, "{:?}", ready.messages);
    ready
        .command
        .clone()
        .expect("a ready sheet shows its command")
}

fn kitty_args(harness: &Harness) -> Vec<String> {
    let windows = harness.launcher.windows();
    assert_eq!(windows.len(), 1, "{windows:?}");
    command::kitty_invocation(&windows[0])
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_jobs_command_removes_every_configured_linear_key_once_in_sorted_order_before_claude() {
    let mut harness = harness(two_keys(), |_| {});
    assert_eq!(
        ready_command(&mut harness),
        format!(
            "kitty --detach --hold --directory {WORKTREE_7} env -u A_LINEAR_KEY -u Z_LINEAR_KEY claude -n {SESSION_7} {PROMPT_7}"
        ),
        "the PR's Owner has no Linear Workflow: every configured Workflow's key goes anyway"
    );

    press(&mut harness, Key::Enter);

    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
    assert_eq!(
        kitty_args(&harness)[..9],
        [
            "--detach",
            "--hold",
            "--directory",
            WORKTREE_7,
            "env",
            "-u",
            "A_LINEAR_KEY",
            "-u",
            "Z_LINEAR_KEY",
        ],
        "the Launcher runs what the sheet showed"
    );
    assert_eq!(kitty_args(&harness)[9], "claude");
}

#[test]
fn two_workflows_naming_the_same_variable_remove_it_once() {
    let mut harness = harness(
        config_with_keys(&[("acme", "DEN_LINEAR_KEY"), ("globex", "DEN_LINEAR_KEY")]),
        |_| {},
    );
    assert_eq!(
        ready_command(&mut harness),
        format!(
            "kitty --detach --hold --directory {WORKTREE_7} env -u DEN_LINEAR_KEY claude -n {SESSION_7} {PROMPT_7}"
        )
    );
}

#[test]
fn the_clipboard_command_removes_the_keys_after_changing_into_the_worktree() {
    let mut harness = harness(two_keys(), |h| h.launcher.outside_kitty());
    let expected = format!(
        "cd {WORKTREE_7} && env -u A_LINEAR_KEY -u Z_LINEAR_KEY claude -n {SESSION_7} {PROMPT_7}"
    );
    assert_eq!(ready_command(&mut harness), expected);

    press(&mut harness, Key::Enter);

    assert_eq!(harness.launcher.clipboard(), [expected]);
    assert!(harness.launcher.windows().is_empty());
}

#[test]
fn reopening_a_live_job_removes_the_same_keys() {
    let mut harness = harness(two_keys(), live_job_7);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Char('o'));
    let ready = sheet(&harness);
    assert_eq!(
        ready.status,
        LaunchStatus::ReopenReady,
        "{:?}",
        ready.messages
    );
    assert_eq!(
        ready.command.as_deref(),
        Some(
            format!(
                "kitty --detach --hold --directory {WORKTREE_7} env -u A_LINEAR_KEY -u Z_LINEAR_KEY claude --resume {SESSION_7}"
            )
            .as_str()
        )
    );

    press(&mut harness, Key::Enter);

    assert_eq!(sheet(&harness).status, LaunchStatus::Reopened);
    assert_eq!(
        harness.launcher.windows()[0].unset_env,
        ["A_LINEAR_KEY", "Z_LINEAR_KEY"]
    );
    assert_eq!(
        kitty_args(&harness)[4..],
        [
            "env",
            "-u",
            "A_LINEAR_KEY",
            "-u",
            "Z_LINEAR_KEY",
            "claude",
            "--resume",
            SESSION_7,
        ]
    );
}

#[test]
fn reopening_outside_kitty_copies_the_resume_command_with_the_keys_removed() {
    let mut harness = harness(two_keys(), |h| {
        live_job_7(h);
        h.launcher.outside_kitty();
    });
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Char('o'));
    press(&mut harness, Key::Enter);
    assert_eq!(
        harness.launcher.clipboard(),
        [format!(
            "cd {WORKTREE_7} && env -u A_LINEAR_KEY -u Z_LINEAR_KEY claude --resume {SESSION_7}"
        )]
    );
}

#[test]
fn github_tokens_are_never_removed() {
    let mut harness = harness(two_keys(), |_| {});
    let shown = ready_command(&mut harness);
    press(&mut harness, Key::Enter);
    let launched = harness.launcher.windows()[0].unset_env.clone();
    for token in ["GH_TOKEN", "GITHUB_TOKEN"] {
        assert!(!shown.contains(token), "{shown}");
        assert!(!launched.iter().any(|v| v == token), "{launched:?}");
    }
}

#[test]
fn without_a_linear_workflow_the_command_is_milestone_3s_byte_for_byte() {
    let mut harness = harness(config_with_keys(&[]), |_| {});
    assert_eq!(
        ready_command(&mut harness),
        format!("kitty --detach --hold --directory {WORKTREE_7} claude -n {SESSION_7} {PROMPT_7}")
    );
    press(&mut harness, Key::Enter);
    assert!(harness.launcher.windows()[0].unset_env.is_empty());
    assert!(!kitty_args(&harness).iter().any(|a| a == "env"));
}
