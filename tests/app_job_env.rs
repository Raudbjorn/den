//! Seam 2: a Job never inherits den's Linear keys (#70, ADR-0007). The command
//! den builds removes every configured Linear Workflow's key variable with
//! `env -u`, written into the command the sheet shows and the Launcher runs,
//! in kitty, on the clipboard and when reopening a Live Job.

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
    handoff::command::{self, WindowMode},
    jobs::{JobRecord, JobSubject},
};
use std::path::PathBuf;
use support::{
    build::{ME, ObservationBuilder, now, pr},
    harness::{Harness, config_with_linear},
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/app";
const WORKTREE: &str = "/home/me/dev/app/.claude/worktrees/feature-7";
const SESSION: &str = "app-pr7-address-review";
const PROMPT: &str = "'/gh-resolve https://github.com/octo/app/pull/7'";
const ZETA_KEY: &str = "DEN_ZETA_LINEAR_KEY";
const ACME_KEY: &str = "ACME_LINEAR_KEY";

/// octo, acme and zeta watched; acme and zeta (never octo, the Job's Owner)
/// each get a Linear Workflow whose key is in `keys` (acme's first). `keys`
/// empty: no Linear Workflow at all. PR 7's Next Step stays octo's Address
/// review, so the removal is shown to cover every Workflow, not only the
/// Job's Owner's.
fn config(keys: &[&str]) -> Config {
    let owners = [("acme", "acme"), ("zeta", "zeta")];
    let mut config = config_with_linear(&["octo", "acme", "zeta"], &owners[..keys.len()]);
    config.main_clone_root = Some(PathBuf::from(ROOT));
    for (workflow, key) in config.linear.iter_mut().zip(keys) {
        (*key).clone_into(&mut workflow.api_key_env);
    }
    config
}

/// octo/app#7, Waiting On me to Address review, in a harness with a Main
/// Clone of octo/app; `arrange` runs before den starts.
fn started(config: Config, arrange: impl FnOnce(&Harness)) -> Harness {
    let mut harness = Harness::new(Ok(config));
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "git@github.com:octo/app.git")]);
    harness.workspace.set_remote_head(
        CLONE,
        "origin",
        "feature-7",
        "0000000000000000000000000000000000000007",
    );
    arrange(&harness);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .owner("acme")
        .owner("zeta")
        .repo("octo/app")
        .pull_request(pr("octo/app", 7).author(ME).changes_requested())
        .build()));
    harness.start();
    harness
}

fn sheet(harness: &Harness) -> &LaunchSheet {
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => sheet,
        other => panic!("no launch sheet: {other:?}"),
    }
}

fn press(harness: &mut Harness, key: Key) {
    harness.settle(Event::Key(key));
}

/// Opens the sheet on PR 7 and returns its command once Ready.
fn ready_command(harness: &mut Harness) -> String {
    press(harness, Key::Char('c'));
    let ready = sheet(harness);
    assert_eq!(ready.status, LaunchStatus::Ready, "{:?}", ready.messages);
    ready.command.clone().expect("a command")
}

/// The arguments the real Launcher would give kitty for the recorded window.
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
fn each_workflows_key_variable_is_removed_once_in_sorted_order_on_the_sheet_and_in_kitty() {
    let mut harness = started(config(&[ZETA_KEY, ACME_KEY]), |_| ());

    let command = ready_command(&mut harness);
    assert_eq!(
        command,
        format!(
            "kitty --detach --hold --directory {WORKTREE} env -u {ACME_KEY} -u {ZETA_KEY} claude -n {SESSION} {PROMPT}"
        )
    );

    press(&mut harness, Key::Enter);
    assert_eq!(
        harness.launcher.windows()[0].removed_env,
        [ACME_KEY, ZETA_KEY]
    );
    let args = kitty_args(&harness);
    let env_at = args
        .iter()
        .position(|a| a == "env")
        .expect("env runs claude");
    assert_eq!(
        args[env_at..env_at + 6],
        ["env", "-u", ACME_KEY, "-u", ZETA_KEY, "claude"]
    );
}

#[test]
fn two_workflows_naming_the_same_variable_remove_it_once() {
    let mut harness = started(config(&[ZETA_KEY, ZETA_KEY]), |_| ());

    let command = ready_command(&mut harness);
    assert_eq!(command.matches("-u ").count(), 1, "{command}");
    assert!(
        command.contains(&format!("env -u {ZETA_KEY} claude -n")),
        "{command}"
    );
}

#[test]
fn the_clipboard_command_removes_the_same_variables() {
    let mut harness = started(config(&[ZETA_KEY]), |h| h.launcher.outside_kitty());

    let command = ready_command(&mut harness);
    let expected = format!("cd {WORKTREE} && env -u {ZETA_KEY} claude -n {SESSION} {PROMPT}");
    assert_eq!(command, expected);

    press(&mut harness, Key::Enter);
    assert_eq!(harness.launcher.clipboard(), [expected]);
}

#[test]
fn reopening_a_live_job_removes_the_same_variables() {
    let item = WorkItemId::PullRequest {
        repo: RepoId::new("octo", "app"),
        number: 7,
    };
    let mut harness = started(config(&[ZETA_KEY]), |h| {
        h.jobs.holding(JobRecord {
            subject: JobSubject::GitHub(item),
            next_step: NextStep::AddressReview,
            target: RepoId::new("octo", "app"),
            main_clone: CLONE.into(),
            worktree: WORKTREE.into(),
            branch: "feature-7".to_owned(),
            session: SESSION.to_owned(),
            launched_at: now(),
            permission_mode: None,
        });
        h.workspace.add_existing(WORKTREE);
        h.workspace
            .register_worktree(CLONE, WORKTREE, Some("feature-7"));
        h.workspace.set_local_branch(
            CLONE,
            "feature-7",
            "2222222222222222222222222222222222222222",
        );
    });

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
                "kitty --detach --hold --directory {WORKTREE} env -u {ZETA_KEY} claude --resume {SESSION}"
            )
            .as_str()
        )
    );

    press(&mut harness, Key::Enter);
    assert_eq!(harness.launcher.windows()[0].removed_env, [ZETA_KEY]);
}

#[test]
fn without_a_linear_workflow_the_command_is_unchanged() {
    let mut harness = started(config(&[]), |_| ());

    let command = ready_command(&mut harness);
    assert_eq!(
        command,
        format!("kitty --detach --hold --directory {WORKTREE} claude -n {SESSION} {PROMPT}")
    );
    assert!(!command.contains("env"), "{command}");
}

#[test]
fn github_tokens_are_never_removed() {
    let mut harness = started(config(&[ZETA_KEY]), |_| ());

    let command = ready_command(&mut harness);
    for token in ["GH_TOKEN", "GITHUB_TOKEN"] {
        assert!(!command.contains(token), "{command}");
    }
}

#[test]
fn the_window_mode_decides_only_where_the_command_goes_not_what_it_removes() {
    // The pure command builder, for both window modes, from one WindowCommand.
    let window = command::WindowCommand {
        directory: WORKTREE.into(),
        session: SESSION.to_owned(),
        prompt: "/gh-resolve x".to_owned(),
        start: command::SessionStart::New,
        permission_mode: None,
        setting_sources: command::SettingSources::All,
        removed_env: vec![ACME_KEY.to_owned(), ZETA_KEY.to_owned()],
    };
    let kitty = command::display(&WindowMode::Kitty, &window);
    let clipboard = command::display(
        &WindowMode::Clipboard {
            why: "not in kitty".to_owned(),
        },
        &window,
    );
    for shown in [kitty, clipboard] {
        assert!(
            shown.contains(&format!(
                "env -u {ACME_KEY} -u {ZETA_KEY} claude -n {SESSION}"
            )),
            "{shown}"
        );
    }
}
