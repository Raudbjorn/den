//! Seam 2, the Add screenshots Job (#79): `c` then `Enter` on Add screenshots
//! launches a Job on the PR head in the approval-required (ask-first)
//! permission mode, with the default `add_screenshots` prompt or the user's
//! own. It is not a Coordination Job (CONTEXT.md, "Add screenshots"): it runs
//! on the viewer's own PR head, so the repo's project settings stay loaded,
//! and it gets the Ground Rules of a Job that changes code (ADR-0008).
//! Asserted on the sheet, the LaunchPlans in Effects and what the fake
//! Launcher and Job store were asked to do; nothing real is launched.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Event, Key,
        view_model::{LaunchSheet, LaunchStatus, Overlay},
    },
    config::Config,
    domain::{NextStep, Observation, RepoId, facts::Screenshots},
    handoff::{
        LaunchPlan, PermissionMode,
        prompts::{self, DEFAULT_ADD_SCREENSHOTS, Prompts},
    },
};
use std::path::{Path, PathBuf};
use support::{
    build::{ME, ObservationBuilder, pr},
    ground_rules::{coding_rules, quoted},
    harness::{Harness, config},
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/api";
const URL_2: &str = "https://github.com/acme/api/pull/2";
/// `pr()` heads are the PR number as 40 hex digits.
const HEAD_2: &str = "0000000000000000000000000000000000000002";
const WORKTREE_2: &str = "/home/me/dev/api/.claude/worktrees/feature-2";

/// `acme` with `acme/api` requiring screenshots, Main Clones under [`ROOT`].
fn screenshots_config() -> Config {
    let mut config = config(&["acme"]);
    config.main_clone_root = Some(PathBuf::from(ROOT));
    config.screenshot_repos = vec![RepoId::new("acme", "api")];
    config
}

/// My #2 in acme/api, ready but for its body: no screenshots.
fn surveyed() -> Observation {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .pull_request(
            pr("acme/api", 2)
                .author(ME)
                .passing("ci")
                .approved()
                .screenshots(Screenshots::Missing),
        )
        .build()
}

fn started(prompts: Result<Prompts, prompts::PromptsError>) -> Harness {
    let config = screenshots_config();
    let mut harness = Harness::new(Ok(config.clone()));
    harness.app = den::app::App::new(Ok(config)).with_launch_settings(prompts, None);
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "git@github.com:acme/api.git")]);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-2", HEAD_2);
    harness.answer(Ok(surveyed()));
    harness.start();
    harness
}

fn press(harness: &mut Harness, key: Key) {
    harness.settle(Event::Key(key));
}

/// Selects #2 and presses `c`.
fn open_sheet(harness: &mut Harness) -> LaunchSheet {
    let row = harness
        .view
        .list
        .rows
        .iter()
        .position(|r| r.repo == "acme/api" && r.reference == "#2")
        .unwrap_or_else(|| panic!("no row #2: {:#?}", harness.view.list.rows));
    for _ in harness.view.list.selected.unwrap_or(0)..row {
        harness.key(Key::Down);
    }
    press(harness, Key::Char('c'));
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => LaunchSheet::clone(sheet),
        other => panic!("no launch sheet: {other:?}"),
    }
}

fn field<'a>(sheet: &'a LaunchSheet, label: &str) -> Option<&'a str> {
    sheet
        .fields
        .iter()
        .find(|f| f.label == label)
        .map(|f| f.value.as_str())
}

fn messages(sheet: &LaunchSheet) -> String {
    sheet
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
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

/// The default template, filled in for #2.
fn default_prompt() -> String {
    DEFAULT_ADD_SCREENSHOTS
        .replace("{repo}", "acme/api")
        .replace("{number}", "2")
        .replace("{url}", URL_2)
        .replace("{branch}", "feature-2")
}

#[test]
fn c_then_enter_on_add_screenshots_launches_an_ask_first_job_on_the_pr_head_with_the_default_prompt()
 {
    let mut harness = started(Ok(Prompts::default()));
    let ready = open_sheet(&mut harness);
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    assert_eq!(ready.next_step, "Add screenshots");
    assert_eq!(field(&ready, "BASE"), Some("PR head feature-2 @ 0000000"));
    assert_eq!(
        field(&ready, "PERMISSIONS"),
        Some("approval-required: Claude asks before acting")
    );
    assert_eq!(
        field(&ready, "PROJECT SETTINGS"),
        None,
        "not a Coordination Job: the viewer's own PR head keeps its settings"
    );
    let prompt = &ready.prompt.as_ref().expect("a prompt").text;
    assert_eq!(*prompt, default_prompt());
    for says in [
        "decide whether this Pull Request changes anything visible",
        "\"No visual change\"",
        "Before | After table",
        "Edit only that section of the Pull Request body",
    ] {
        assert!(prompt.contains(says), "{says:?} not in {prompt:?}");
    }
    let command = ready.command.as_deref().expect("a command");
    assert!(command.contains("--permission-mode manual"), "{command}");
    assert!(!command.contains("--setting-sources user"), "{command}");
    assert!(
        command.contains(&format!(
            "--append-system-prompt {}",
            quoted(&coding_rules("acme/api#2"))
        )),
        "the coding Ground Rules, naming only its own PR: {command}"
    );

    press(&mut harness, Key::Enter);

    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1, "{plans:?}");
    let plan = &plans[0];
    assert_eq!(plan.next_step, NextStep::AddScreenshots);
    assert_eq!(plan.permission_mode, Some(PermissionMode::ApprovalRequired));
    assert_eq!(plan.base.commit, HEAD_2);
    assert_eq!(plan.worktree, PathBuf::from(WORKTREE_2));
    assert_eq!(plan.prompt, default_prompt());
    let windows = harness.launcher.windows();
    assert_eq!(windows.len(), 1);
    assert_eq!(
        windows[0].permission_mode,
        Some(PermissionMode::ApprovalRequired)
    );
    assert_eq!(windows[0].ground_rules, coding_rules("acme/api#2"));
    let jobs = harness.jobs.records();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].next_step, NextStep::AddScreenshots);
    assert_eq!(
        jobs[0].permission_mode,
        Some(PermissionMode::ApprovalRequired),
        "recorded, so reopening the Job asks again"
    );
}

#[test]
fn the_users_add_screenshots_template_replaces_the_default() {
    let custom = prompts::parse(
        Path::new("prompts.toml"),
        "add_screenshots = \"/shoot {url} on {branch}\"\n",
    )
    .expect("add_screenshots is a prompt kind");
    let mut harness = started(Ok(custom));
    let ready = open_sheet(&mut harness);
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    let expected = format!("/shoot {URL_2} on feature-2");
    assert_eq!(ready.prompt.as_ref().expect("a prompt").text, expected);

    press(&mut harness, Key::Enter);

    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1, "{plans:?}");
    assert_eq!(plans[0].prompt, expected);
    assert_eq!(
        plans[0].permission_mode,
        Some(PermissionMode::ApprovalRequired),
        "a custom template never changes the mode"
    );
    assert_eq!(
        harness.launcher.windows()[0].ground_rules,
        coding_rules("acme/api#2"),
        "nor removes the Ground Rules"
    );
}
