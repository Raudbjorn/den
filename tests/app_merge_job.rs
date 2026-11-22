//! Seam 2, Merge as a Coordination Job (#72, spec 0004 stories 21–25): Merge
//! launches ask-first, pinned to the head den observed (`{head}`,
//! `--match-head-commit`), with the Repo's `merge_method` or the instruction
//! to use what the repository allows. A head that moved before launch is
//! refused, as a Land member's is: den never merges a head it didn't decide
//! on. A custom `merge` template must use `{head}`.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Key,
        view_model::{LaunchSheet, LaunchStatus, Overlay},
    },
    config::Config,
    domain::{MergeMethod, MergeMethods, NextStep, RepoId},
    handoff::{
        LaunchPlan, PermissionMode,
        prompts::{self, PromptKind},
    },
};
use std::{collections::BTreeMap, path::Path, path::PathBuf};
use support::{
    build::{ME, ObservationBuilder, pr},
    ground_rules::{common_rules, quoted},
    harness::Harness,
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/app";
const URL_7: &str = "https://github.com/octo/app/pull/7";
/// The default head commit of PR 7 (`pr()` uses the number in hex).
const HEAD_7: &str = "0000000000000000000000000000000000000007";
const MOVED_HEAD: &str = "1111111111111111111111111111111111111111";
/// What `{merge_method}` says for a Repo without one.
const ASK: &str =
    "the merge method the repository allows; if it allows more than one, ask me which to use";

fn config(merge_methods: MergeMethods) -> Config {
    Config {
        owners: vec!["octo".to_owned()],
        main_clone_root: Some(PathBuf::from(ROOT)),
        main_clones: BTreeMap::new(),
        linear: Vec::new(),
        skips: den::domain::Skips::default(),
        merge_methods,
        screenshot_repos: Vec::new(),
        refresh_interval_secs: None,
        legacy_marker_prefixes: Vec::new(),
    }
}

/// octo/app#7, mine, approved with passing checks: Merge.
fn started(config: Config) -> Harness {
    let mut harness = Harness::new(Ok(config));
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(pr("octo/app", 7).author(ME).approved().passing("ci"))
        .build()));
    harness.start();
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "git@github.com:octo/app.git")]);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-7", HEAD_7);
    harness
}

fn press(harness: &mut Harness, key: Key) {
    harness.settle(den::app::Event::Key(key));
}

fn sheet(harness: &Harness) -> &LaunchSheet {
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => sheet,
        other => panic!("no launch sheet: {other:?}"),
    }
}

fn prompt(sheet: &LaunchSheet) -> &str {
    &sheet.prompt.as_ref().expect("a prompt").text
}

fn messages(sheet: &LaunchSheet) -> String {
    sheet
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn field<'a>(sheet: &'a LaunchSheet, label: &str) -> Option<(&'a str, Option<&'a str>)> {
    sheet
        .fields
        .iter()
        .find(|f| f.label == label)
        .map(|f| (f.value.as_str(), f.note.as_deref()))
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

fn merge_prompt(method: &str) -> String {
    format!(
        "Pull Request octo/app#7 ({URL_7}) is approved and its checks pass. You are on its head branch feature-7. Check that it is ready, then merge it pinned to the head den observed: gh pr merge {URL_7} --match-head-commit {HEAD_7}, using {method}. If GitHub refuses because the head moved, stop and tell me; never merge another head."
    )
}

/// Acceptance: Merge shows and runs the ask-first mode; its default prompt
/// merges with `--match-head-commit <head>`; without a `merge_method` it is
/// told to use what the repository allows and ask if several are.
#[test]
fn merge_launches_ask_first_pinned_to_the_head_den_observed() {
    let mut harness = started(config(MergeMethods::default()));
    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    assert_eq!(ready.next_step, "Merge");
    assert_eq!(
        field(&ready, "PERMISSIONS").map(|f| f.0),
        Some("approval-required: Claude asks before acting")
    );
    assert_eq!(prompt(&ready), merge_prompt(ASK));
    let command = ready.command.clone().expect("a command");
    assert!(command.contains("--permission-mode manual"), "{command}");
    // #71's common-only rules must compose with #72's ask-first Merge mode.
    let rules = common_rules("octo/app#7");
    assert!(
        command.contains(&format!("--append-system-prompt {} -n ", quoted(&rules))),
        "{command}"
    );

    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1, "{}", messages(sheet(&harness)));
    assert_eq!(plans[0].next_step, NextStep::Merge);
    assert_eq!(
        plans[0].permission_mode,
        Some(PermissionMode::ApprovalRequired)
    );
    assert_eq!(plans[0].prompt, merge_prompt(ASK));
    assert_eq!(harness.launcher.windows()[0].ground_rules, rules);
    assert_eq!(
        harness.launcher.windows()[0].permission_mode,
        Some(PermissionMode::ApprovalRequired)
    );
    assert_eq!(
        harness.jobs.records()[0].permission_mode,
        Some(PermissionMode::ApprovalRequired)
    );
}

/// Acceptance: a Repo's `merge_method` is the Merge prompt's flag.
#[test]
fn a_repos_merge_method_is_the_merge_prompts_flag() {
    for (method, flag) in [
        (MergeMethod::Squash, "--squash"),
        (MergeMethod::Merge, "--merge"),
        (MergeMethod::Rebase, "--rebase"),
    ] {
        let mut harness = started(config(
            MergeMethods::default().with(RepoId::new("Octo", "App"), method),
        ));
        press(&mut harness, Key::Char('c'));
        assert_eq!(prompt(sheet(&harness)), merge_prompt(flag), "{method:?}");
    }
}

/// Story 22: a push after den looked is never merged unseen. A head that
/// moved on GitHub before launch refuses, like a Land member's, and nothing
/// is fetched, added, opened or recorded.
#[test]
fn a_merge_whose_head_moved_before_launch_is_refused() {
    let mut harness = started(config(MergeMethods::default()));
    press(&mut harness, Key::Char('c'));
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-7", MOVED_HEAD);
    press(&mut harness, Key::Enter);

    let refused = sheet(&harness).clone();
    assert_eq!(
        refused.status,
        LaunchStatus::Refused,
        "{}",
        messages(&refused)
    );
    assert!(
        messages(&refused).contains(
            "octo/app#7's head is now 1111111 on GitHub, not 0000000, which den decided to Merge; nothing was done"
        ),
        "{}",
        messages(&refused)
    );
    assert!(harness.launcher.windows().is_empty());
    assert!(harness.jobs.records().is_empty());
}

/// Acceptance: a custom `merge` template without `{head}` makes only Merge
/// unusable, naming the file and line; every other kind loads.
#[test]
fn a_merge_template_without_head_makes_only_merge_unusable() {
    let path = Path::new("/home/me/.config/den/prompts.toml");
    let loaded = prompts::parse(
        path,
        "fix_ci = \"/fix {url}\"\nmerge = \"gh pr merge {url} {merge_method}\"\n",
    )
    .expect("well formed");
    let unusable = loaded
        .unusable(NextStep::Merge)
        .expect("merge is unusable")
        .to_string();
    assert!(
        unusable.contains("prompts.toml, line 2") && unusable.contains("`merge` must use {head}"),
        "{unusable}"
    );
    assert_eq!(loaded.unusable(NextStep::FixCi), None);
    assert_eq!(loaded.template(NextStep::FixCi), "/fix {url}");

    let pinned = prompts::parse(
        path,
        "merge = \"gh pr merge {url} --match-head-commit {head}\"\n",
    )
    .expect("well formed");
    assert_eq!(pinned.unusable(NextStep::Merge), None);
}

/// `{head}` is Merge's alone; `{merge_method}` is Merge's and Land's.
#[test]
fn head_and_merge_method_belong_to_merge_and_land() {
    let path = Path::new("/home/me/.config/den/prompts.toml");
    let error = prompts::parse(path, "fix_ci = \"/fix {url} {head}\"\n")
        .expect_err("fix_ci can't use {head}")
        .to_string();
    assert!(
        error.contains("the merge-only placeholder {head}"),
        "{error}"
    );
    let error = prompts::parse(path, "fix_ci = \"/fix {url} {merge_method}\"\n")
        .expect_err("fix_ci can't use {merge_method}")
        .to_string();
    assert!(
        error.contains("a placeholder only merge and land have: {merge_method}"),
        "{error}"
    );
    let land = prompts::parse(path, "land = \"/land {members}, using {merge_method}\"\n")
        .expect("land may use {merge_method}");
    assert_eq!(land.unusable(PromptKind::Step(NextStep::Land)), None);
}

/// Review on #88: a head accepted for another kind (`HeadMoved`, then ⏎)
/// never pins a Merge. Merge never offers to accept a moved head, so the
/// sheet's accepted head, older than what den observed since, would refuse
/// every Merge launch as moved. The Merge is pinned to the head the latest
/// survey observed.
#[test]
fn a_head_accepted_for_another_kind_never_pins_a_merge() {
    const THIRD_HEAD: &str = "3333333333333333333333333333333333333333";
    let mut harness = Harness::new(Ok(config(MergeMethods::default())));
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(pr("octo/app", 7).author(ME).changes_requested())
        .build()));
    harness.start();
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "git@github.com:octo/app.git")]);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-7", HEAD_7);

    // A refresh is under way; its answer has #7 approved at a third head.
    let survey = harness.key(Key::Char('r'));
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(
            pr("octo/app", 7)
                .author(ME)
                .head(THIRD_HEAD)
                .approved()
                .passing("ci"),
        )
        .build()));

    // Address review: the head moved, and the user accepts it.
    press(&mut harness, Key::Char('c'));
    assert_eq!(sheet(&harness).next_step, "Address review");
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-7", MOVED_HEAD);
    press(&mut harness, Key::Enter);
    assert_eq!(sheet(&harness).status, LaunchStatus::HeadMoved);

    // The refresh lands: #7 is Merge now, at the head den just observed.
    for effect in &survey {
        harness.perform(effect);
    }
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-7", THIRD_HEAD);
    press(&mut harness, Key::Enter);
    let asked = sheet(&harness).clone();
    assert_eq!(asked.next_step, "Merge", "{}", messages(&asked));
    assert!(
        prompt(&asked).contains(&format!("--match-head-commit {THIRD_HEAD}")),
        "{}",
        prompt(&asked)
    );

    press(&mut harness, Key::Enter);
    let launched = sheet(&harness).clone();
    assert_ne!(
        launched.status,
        LaunchStatus::Refused,
        "{}",
        messages(&launched)
    );
    let plans = launch_plans(&harness);
    let merge = plans
        .iter()
        .find(|plan| plan.next_step == NextStep::Merge)
        .expect("a Merge plan");
    assert_eq!(merge.base.commit, THIRD_HEAD);
    assert_eq!(harness.launcher.windows().len(), 1);
}
