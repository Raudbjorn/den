//! Seam 2: launching Jobs for Issues, Branches and Runs, cloning a missing
//! Main Clone, and suffixing a worktree name another Work Item's Job has
//! (#12). Keys go in; the tests assert on the launch sheet, the LaunchPlans in
//! Effects, and what the fake Workspace, Launcher and Job store were asked to do.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Event, Key,
        view_model::{LaunchSheet, LaunchStatus, Overlay},
    },
    config::Config,
    domain::{NextStep, Observation, Repo},
    handoff::{
        BaseRef, BranchStart, LaunchPlan, SettingSources,
        prompts::{self, PromptFields, Prompts},
    },
    ports::{WorktreeAdd, WorktreeStart},
};
use std::{collections::BTreeMap, path::PathBuf};
use support::{
    build::{ME, ObservationBuilder, branch, issue, repo_id, run},
    fakes::WorkspaceCall,
    harness::Harness,
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/app";
const OWNER_CLONE: &str = "/home/me/dev/octo-app";
const URL: &str = "https://github.com/octo/app";
const MAIN_TIP: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MOVED_TIP: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BRANCH_TIP: &str = "cccccccccccccccccccccccccccccccccccccccc";
const ISSUE_BRANCH: &str = "implement_9-add-a-login-page";

fn launch_config() -> Config {
    Config {
        owners: vec!["octo".to_owned()],
        main_clone_root: Some(PathBuf::from(ROOT)),
        main_clones: BTreeMap::new(),
        linear: Vec::new(),
        skips: den::domain::Skips::default(),
    }
}

/// octo/app with an Issue to Implement, an Issue to Triage, a Branch to open
/// a PR for, and a failed default-branch Run.
fn observation() -> ObservationBuilder {
    ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .issue(
            issue("octo/app", 9)
                .title("Add a login page!")
                .assignee(ME)
                .label("feature"),
        )
        .issue(issue("octo/app", 11).title("Crash on start"))
        .branch(branch("octo/app", "feature/login").author(ME))
        .run(run("octo/app", "ci.yml", 41).failed().by(ME))
}

fn started(observation: Observation) -> Harness {
    started_with(observation, Ok(Prompts::default()))
}

fn started_with(
    observation: Observation,
    prompts: Result<Prompts, prompts::PromptsError>,
) -> Harness {
    let mut harness = Harness::new(Ok(launch_config()));
    harness.app = den::app::App::new(Ok(launch_config())).with_launch_settings(prompts, None);
    harness.answer(Ok(observation));
    harness.start();
    harness
}

/// A Main Clone of octo/app whose `origin` points at it, `main` at MAIN_TIP.
fn with_clone(harness: &Harness) {
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "git@github.com:octo/app.git")]);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "main", MAIN_TIP);
}

fn press(harness: &mut Harness, key: Key) {
    harness.settle(Event::Key(key));
}

/// Selects the row for `reference` (as `WorkItemId` displays it) and presses `c`.
fn open_sheet(harness: &mut Harness, reference: &str) {
    let rows: Vec<String> = harness
        .view
        .list
        .rows
        .iter()
        .map(|r| r.id.to_string())
        .collect();
    let index = rows
        .iter()
        .position(|r| r == reference)
        .unwrap_or_else(|| panic!("no row {reference} in {rows:?}"));
    while harness.view.list.selected != Some(index) {
        let before = harness.view.list.selected;
        let key = if before.is_some_and(|s| s > index) {
            Key::Up
        } else {
            Key::Down
        };
        press(harness, key);
        assert_ne!(before, harness.view.list.selected, "selection stuck");
    }
    press(harness, Key::Char('c'));
}

fn sheet(harness: &Harness) -> &LaunchSheet {
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => sheet,
        other => panic!("no launch sheet: {other:?}"),
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

fn worktree(clone: &str, name: &str) -> PathBuf {
    PathBuf::from(clone)
        .join(".claude")
        .join("worktrees")
        .join(name)
}

fn assert_ready(harness: &Harness) {
    let sheet = sheet(harness);
    assert_eq!(sheet.status, LaunchStatus::Ready, "{}", messages(sheet));
}

#[test]
fn an_issue_launch_plans_a_type_slug_branch_from_the_targets_default_branch() {
    let mut harness = started(observation().build());
    with_clone(&harness);

    open_sheet(&mut harness, "octo/app#9");
    assert_ready(&harness);
    let ready = sheet(&harness).clone();
    assert_eq!(ready.next_step, "Implement");
    assert_eq!(field(&ready, "BASE").0, "default branch main @ aaaaaaa");
    assert_eq!(
        field(&ready, "BRANCH"),
        (
            ISSUE_BRANCH,
            Some("new, from origin/main; tracks nothing until pushed")
        )
    );
    let expected_worktree = worktree(CLONE, ISSUE_BRANCH);
    assert_eq!(
        field(&ready, "WORKTREE"),
        (expected_worktree.to_str().unwrap(), Some("free"))
    );
    assert_eq!(field(&ready, "SESSION").0, "app-issue9-implement");
    assert_eq!(
        ready.prompt.as_ref().expect("a prompt").text,
        "/implement https://github.com/octo/app/issues/9"
    );

    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1, "{plans:?}");
    assert_eq!(
        plans[0].base.reference,
        BaseRef::DefaultBranch("main".to_owned())
    );
    assert_eq!(plans[0].base.commit, MAIN_TIP);
    assert_eq!(plans[0].branch.start, BranchStart::Fresh);
    assert_eq!(
        harness.workspace.calls(),
        [
            WorkspaceCall::Fetch {
                clone: CLONE.into(),
                remote: "origin".to_owned(),
                refspec: "+refs/heads/main:refs/remotes/origin/main".to_owned(),
            },
            WorkspaceCall::AddWorktree {
                clone: CLONE.into(),
                add: WorktreeAdd {
                    path: expected_worktree.clone(),
                    branch: ISSUE_BRANCH.to_owned(),
                    // Not tracking main: pushing creates the new branch.
                    start: WorktreeStart::NewBranch {
                        start: "refs/remotes/origin/main".to_owned()
                    },
                },
            },
        ]
    );
    // An Issue Job starts from the default branch, not a PR head: every
    // setting source, as in milestone 1.
    assert!(!ready.fields.iter().any(|f| f.label == "PROJECT SETTINGS"));
    assert_eq!(
        harness.launcher.windows()[0].setting_sources,
        SettingSources::All
    );
    let jobs = harness.jobs.records();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].branch, ISSUE_BRANCH);
    assert_eq!(jobs[0].worktree, expected_worktree);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
}

#[test]
fn a_triage_issue_gets_a_triage_branch_and_the_triage_prompt() {
    let mut harness = started(observation().build());
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app#11");
    assert_ready(&harness);
    let ready = sheet(&harness);
    assert_eq!(ready.next_step, "Triage");
    assert_eq!(field(ready, "BRANCH").0, "triage_11-crash-on-start");
    assert_eq!(
        ready.prompt.as_ref().expect("a prompt").text,
        "/triage https://github.com/octo/app/issues/11"
    );
}

#[test]
fn a_branch_launch_works_on_the_existing_branch() {
    let mut harness = started(observation().build());
    with_clone(&harness);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature/login", BRANCH_TIP);

    open_sheet(&mut harness, "octo/app:feature/login");
    assert_ready(&harness);
    let ready = sheet(&harness).clone();
    assert_eq!(ready.next_step, "Open PR");
    assert_eq!(field(&ready, "BASE").0, "branch feature/login @ ccccccc");
    assert_eq!(
        field(&ready, "BRANCH"),
        ("feature/login", Some("new, tracking origin/feature/login"))
    );
    let prompt = ready.prompt.as_ref().expect("a prompt").text.clone();
    assert!(!prompt.starts_with('/'), "{prompt}");
    assert!(
        prompt.contains("feature/login") && prompt.contains("octo/app"),
        "{prompt}"
    );
    assert_eq!(field(&ready, "SESSION").0, "app-feature-login-open-pr");

    press(&mut harness, Key::Enter);
    assert_eq!(
        harness.workspace.calls(),
        [
            WorkspaceCall::Fetch {
                clone: CLONE.into(),
                remote: "origin".to_owned(),
                refspec: "+refs/heads/feature/login:refs/remotes/origin/feature/login".to_owned(),
            },
            WorkspaceCall::AddWorktree {
                clone: CLONE.into(),
                add: WorktreeAdd {
                    path: worktree(CLONE, "feature-login"),
                    branch: "feature/login".to_owned(),
                    start: WorktreeStart::NewBranchTracking {
                        remote: "origin".to_owned(),
                        start: "refs/remotes/origin/feature/login".to_owned()
                    },
                },
            },
        ]
    );
    // A Branch Job is on the Repo's own branch: every setting source.
    assert!(!ready.fields.iter().any(|f| f.label == "PROJECT SETTINGS"));
    assert_eq!(
        harness.launcher.windows()[0].setting_sources,
        SettingSources::All
    );
}

#[test]
fn a_branch_launch_reuses_a_local_branch_at_the_tip_and_refuses_one_elsewhere() {
    let mut harness = started(observation().build());
    with_clone(&harness);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature/login", BRANCH_TIP);
    harness
        .workspace
        .set_local_branch(CLONE, "feature/login", MAIN_TIP);
    open_sheet(&mut harness, "octo/app:feature/login");
    assert_eq!(
        messages(sheet(&harness)),
        "a local branch feature/login already exists at aaaaaaa, not at origin/feature/login ccccccc; den won't move it"
    );
    press(&mut harness, Key::Esc);

    harness
        .workspace
        .set_local_branch(CLONE, "feature/login", BRANCH_TIP);
    open_sheet(&mut harness, "octo/app:feature/login");
    assert_ready(&harness);
    assert_eq!(
        field(sheet(&harness), "BRANCH").1,
        Some("existing local branch, already at the base")
    );
}

#[test]
fn a_branch_gone_from_the_remote_is_refused() {
    let mut harness = started(observation().build());
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app:feature/login");
    assert_eq!(
        messages(sheet(&harness)),
        "the branch feature/login is not on origin"
    );
}

#[test]
fn a_run_launch_plans_a_new_branch_from_the_default_branch() {
    let mut harness = started(observation().build());
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app workflow .github/workflows/ci.yml");
    assert_ready(&harness);
    let ready = sheet(&harness).clone();
    assert_eq!(ready.next_step, "Fix CI");
    assert_eq!(field(&ready, "BASE").0, "default branch main @ aaaaaaa");
    assert_eq!(field(&ready, "BRANCH").0, "fix-ci_ci");
    assert_eq!(field(&ready, "SESSION").0, "app-ci-fix-ci");
    assert_eq!(
        ready.prompt.as_ref().expect("a prompt").text,
        "/diagnosing-bugs CI is failing for https://github.com/octo/app/actions/runs/41. You are on branch fix-ci_ci. Find out why and fix it."
    );

    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].base.reference,
        BaseRef::DefaultBranch("main".to_owned())
    );
    assert_eq!(plans[0].branch.start, BranchStart::Fresh);
    assert_eq!(plans[0].worktree, worktree(CLONE, "fix-ci_ci"));
}

#[test]
fn a_default_branch_that_moved_between_sheet_and_launch_asks_again() {
    let mut harness = started(observation().build());
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app#9");
    assert_ready(&harness);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "main", MOVED_TIP);

    press(&mut harness, Key::Enter);
    let asked = sheet(&harness).clone();
    assert_eq!(asked.status, LaunchStatus::HeadMoved);
    assert_eq!(
        asked.headline,
        "The default branch main moved from aaaaaaa to bbbbbbb: check, then ⏎ launches on bbbbbbb"
    );
    assert!(harness.workspace.calls().is_empty());

    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[1].base.commit, MOVED_TIP);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
}

#[test]
fn an_empty_repo_has_no_default_branch_to_start_from() {
    let mut harness = started(
        ObservationBuilder::new()
            .owner("octo")
            .with_repo(Repo {
                id: repo_id("octo/app"),
                url: URL.to_owned(),
                is_fork: false,
                is_archived: false,
                default_branch: None,
                parent: None,
            })
            .issue(issue("octo/app", 9).assignee(ME).label("feature"))
            .build(),
    );
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app#9");
    assert_eq!(
        messages(sheet(&harness)),
        "octo/app is empty: it has no default branch to start a new branch from"
    );
    assert!(harness.workspace.calls().is_empty());
}

// ---- a missing Main Clone ----

#[test]
fn a_missing_main_clone_plans_a_clone_into_the_repos_name_under_the_root() {
    let mut harness = started(observation().build());
    harness
        .workspace
        .add_existing("/home/me/dev/other/notes.txt");
    harness.workspace.host(URL, "main", MAIN_TIP);

    open_sheet(&mut harness, "octo/app#9");
    assert_ready(&harness);
    let ready = sheet(&harness).clone();
    assert_eq!(
        ready.headline,
        "Ready: ⏎ clones octo/app, then launches; esc cancels"
    );
    assert_eq!(
        field(&ready, "MAIN CLONE"),
        (
            CLONE,
            Some("not cloned yet: launching clones https://github.com/octo/app here")
        )
    );
    assert_eq!(
        field(&ready, "REMOTE").0,
        "origin → https://github.com/octo/app"
    );
    assert_eq!(field(&ready, "BASE").0, "default branch main @ aaaaaaa");
    assert!(
        harness.workspace.calls().is_empty(),
        "nothing is cloned before Enter"
    );

    press(&mut harness, Key::Enter);
    let issue_worktree = worktree(CLONE, ISSUE_BRANCH);
    assert_eq!(
        harness.workspace.calls(),
        [
            WorkspaceCall::Clone {
                url: URL.to_owned(),
                remote: "origin".to_owned(),
                into: CLONE.into(),
            },
            WorkspaceCall::Fetch {
                clone: CLONE.into(),
                remote: "origin".to_owned(),
                refspec: "+refs/heads/main:refs/remotes/origin/main".to_owned(),
            },
            WorkspaceCall::AddWorktree {
                clone: CLONE.into(),
                add: WorktreeAdd {
                    path: issue_worktree.clone(),
                    branch: ISSUE_BRANCH.to_owned(),
                    start: WorktreeStart::NewBranch {
                        start: "refs/remotes/origin/main".to_owned()
                    },
                },
            },
        ]
    );
    let done = sheet(&harness);
    assert_eq!(done.status, LaunchStatus::Launched, "{}", messages(done));
    assert!(
        messages(done).contains("cloned https://github.com/octo/app into /home/me/dev/app"),
        "{}",
        messages(done)
    );
    let jobs = harness.jobs.records();
    assert_eq!(jobs[0].main_clone, PathBuf::from(CLONE));
    assert_eq!(jobs[0].worktree, issue_worktree);
}

#[test]
fn a_missing_main_clone_falls_back_to_owner_dash_repo_when_the_repos_name_is_taken() {
    let mut harness = started(observation().build());
    // ~/dev/app is someone else's app.
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "https://github.com/other/app")]);
    harness.workspace.host(URL, "main", MAIN_TIP);

    open_sheet(&mut harness, "octo/app#9");
    assert_ready(&harness);
    assert_eq!(
        field(sheet(&harness), "MAIN CLONE"),
        (
            OWNER_CLONE,
            Some(
                "not cloned yet: launching clones https://github.com/octo/app here; /home/me/dev/app is taken"
            )
        )
    );
    press(&mut harness, Key::Enter);
    assert_eq!(
        harness.workspace.calls()[0],
        WorkspaceCall::Clone {
            url: URL.to_owned(),
            remote: "origin".to_owned(),
            into: OWNER_CLONE.into(),
        }
    );
    assert_eq!(
        launch_plans(&harness)[0].worktree,
        worktree(OWNER_CLONE, ISSUE_BRANCH)
    );
}

#[test]
fn a_missing_main_clone_is_refused_when_both_folders_exist_and_neither_is_the_repo() {
    let mut harness = started(observation().build());
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "https://github.com/other/app")]);
    harness
        .workspace
        .add_existing("/home/me/dev/octo-app/README");
    harness.workspace.host(URL, "main", MAIN_TIP);

    open_sheet(&mut harness, "octo/app#9");
    let refused = sheet(&harness);
    assert_eq!(refused.status, LaunchStatus::Refused);
    let text = messages(refused);
    assert!(
        text.contains(
            "den won't clone octo/app into a folder that exists: /home/me/dev/app and /home/me/dev/octo-app both exist"
        ),
        "{text}"
    );
    assert!(
        text.contains("/home/me/dev/app is named like it but its remotes are origin → https://github.com/other/app"),
        "{text}"
    );
    press(&mut harness, Key::Enter);
    assert!(launch_plans(&harness).is_empty());
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn a_clone_folder_den_cannot_inspect_is_never_treated_as_free_or_skipped() {
    let mut harness = started(observation().build());
    harness
        .workspace
        .add_existing("/home/me/dev/other/notes.txt");
    harness.workspace.make_unknown(CLONE, "permission denied");
    harness.workspace.host(URL, "main", MAIN_TIP);
    open_sheet(&mut harness, "octo/app#9");
    let text = messages(sheet(&harness));
    assert!(
        text.contains(
            "den can't tell whether /home/me/dev/app is free to clone into: permission denied"
        ),
        "{text}"
    );
}

#[test]
fn a_clone_root_den_cannot_write_in_is_refused() {
    let mut harness = started(observation().build());
    harness.workspace.add_unwritable_folder(ROOT);
    harness.workspace.host(URL, "main", MAIN_TIP);
    open_sheet(&mut harness, "octo/app#9");
    let text = messages(sheet(&harness));
    assert!(
        text.contains("den can't clone into /home/me/dev: den can't write in it"),
        "{text}"
    );
}

#[test]
fn a_folder_that_appears_before_launch_is_never_cloned_into() {
    let mut harness = started(observation().build());
    harness
        .workspace
        .add_existing("/home/me/dev/other/notes.txt");
    harness.workspace.host(URL, "main", MAIN_TIP);
    open_sheet(&mut harness, "octo/app#9");
    assert_ready(&harness);
    harness.workspace.add_existing(CLONE);

    press(&mut harness, Key::Enter);
    let refused = sheet(&harness);
    assert_eq!(
        refused.status,
        LaunchStatus::Refused,
        "{}",
        messages(refused)
    );
    assert_eq!(
        messages(refused),
        "/home/me/dev/app appeared since the sheet opened; den won't clone into it. Open the sheet again"
    );
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn a_repo_den_cannot_reach_to_clone_is_refused_at_the_sheet() {
    let mut harness = started(observation().build());
    harness
        .workspace
        .add_existing("/home/me/dev/other/notes.txt");
    // Nothing hosted at URL: asking it fails.
    open_sheet(&mut harness, "octo/app#9");
    let text = messages(sheet(&harness));
    assert!(
        text.starts_with("den couldn't ask https://github.com/octo/app where main is:"),
        "{text}"
    );
}

#[test]
fn a_pr_with_no_main_clone_is_cloned_too_and_starts_on_its_head() {
    let mut harness = started(
        ObservationBuilder::new()
            .owner("octo")
            .repo("octo/app")
            .pull_request(
                support::build::pr("octo/app", 7)
                    .author(ME)
                    .changes_requested(),
            )
            .build(),
    );
    harness
        .workspace
        .add_existing("/home/me/dev/other/notes.txt");
    let head = "0000000000000000000000000000000000000007";
    harness.workspace.host(URL, "feature-7", head);
    open_sheet(&mut harness, "octo/app#7");
    assert_ready(&harness);
    assert_eq!(
        field(sheet(&harness), "BASE").0,
        "PR head feature-7 @ 0000000"
    );
    press(&mut harness, Key::Enter);
    let calls = harness.workspace.calls();
    assert_eq!(
        calls[0],
        WorkspaceCall::Clone {
            url: URL.to_owned(),
            remote: "origin".to_owned(),
            into: CLONE.into()
        }
    );
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
}

// ---- worktree-name collisions ----

/// Two workflows both named "CI" (`ci.yml`, `nightly.yml`) are two Work Items
/// whose new branches would both be `fix-ci_ci`.
fn two_ci_workflows() -> Observation {
    ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .run(run("octo/app", "ci.yml", 41).named("CI").failed().by(ME))
        .run(
            run("octo/app", "nightly.yml", 42)
                .named("CI")
                .failed()
                .by(ME),
        )
        .build()
}

#[test]
fn a_worktree_name_another_work_items_job_has_gets_a_numeric_suffix_shown_on_the_sheet() {
    let mut harness = started(two_ci_workflows());
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app workflow .github/workflows/ci.yml");
    assert_ready(&harness);
    press(&mut harness, Key::Enter);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
    press(&mut harness, Key::Esc);

    open_sheet(
        &mut harness,
        "octo/app workflow .github/workflows/nightly.yml",
    );
    assert_ready(&harness);
    let ready = sheet(&harness).clone();
    let suffixed = worktree(CLONE, "fix-ci_ci-2");
    assert_eq!(
        field(&ready, "WORKTREE"),
        (
            suffixed.to_str().unwrap(),
            Some(
                "free; fix-ci_ci is taken by the Job for octo/app workflow .github/workflows/ci.yml, so den added a number"
            )
        )
    );
    assert_eq!(field(&ready, "BRANCH").0, "fix-ci_ci-2");
    // Integration (decision a): the session carries the same suffix as the
    // worktree, by the one rule shared with #14's fresh start.
    assert_eq!(field(&ready, "SESSION").0, "app-nightly-fix-ci-2");
    assert!(
        ready
            .prompt
            .as_ref()
            .expect("a prompt")
            .text
            .contains("You are on branch fix-ci_ci-2.")
    );

    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    let plan = plans.last().expect("a second plan");
    assert_eq!(plan.worktree, suffixed);
    assert_eq!(plan.branch.name, "fix-ci_ci-2");
    let renamed = plan.renamed.as_ref().expect("renamed");
    assert_eq!(renamed.wanted, "fix-ci_ci");
    assert_eq!(harness.jobs.records().len(), 2);
    assert_eq!(harness.jobs.records()[1].worktree, suffixed);
}

#[test]
fn the_same_work_items_own_job_is_never_suffixed_around() {
    let mut harness = started(two_ci_workflows());
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app workflow .github/workflows/ci.yml");
    press(&mut harness, Key::Enter);
    press(&mut harness, Key::Esc);

    // Launching ci.yml again is the Live Job question (#14), not a collision.
    // Integration (#12 + #14): before #14 this sheet was a WorktreeExists
    // refusal naming the Job; with #14 merged it asks "open existing or start
    // fresh?" about that same, unsuffixed worktree. The boundary this test
    // pins is unchanged: den never suffixes around the Work Item's own Job.
    open_sheet(&mut harness, "octo/app workflow .github/workflows/ci.yml");
    let again = sheet(&harness);
    assert_eq!(again.status, LaunchStatus::LiveJob);
    let unsuffixed = worktree(CLONE, "fix-ci_ci");
    assert_eq!(field(again, "JOB").0, unsuffixed.to_str().unwrap());
    assert_eq!(field(again, "SESSION").0, "app-ci-fix-ci");
}

#[test]
fn one_suffix_rule_names_a_collision_and_a_fresh_start_alike() {
    // Decision a: a name another Work Item's Job has (#12) and a fresh start
    // beside this Work Item's Live Job (#14) are both `<name>-<n>`, on the
    // worktree, the branch and the session alike.
    let mut harness = started(two_ci_workflows());
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app workflow .github/workflows/ci.yml");
    press(&mut harness, Key::Enter);
    press(&mut harness, Key::Esc);
    open_sheet(
        &mut harness,
        "octo/app workflow .github/workflows/nightly.yml",
    );
    press(&mut harness, Key::Enter);
    press(&mut harness, Key::Esc);
    let collision = launch_plans(&harness).last().cloned().expect("a plan");
    assert_eq!(
        (
            collision.worktree,
            collision.branch.name.as_str(),
            collision.session.as_str()
        ),
        (
            worktree(CLONE, "fix-ci_ci-2"),
            "fix-ci_ci-2",
            "app-nightly-fix-ci-2"
        )
    );

    // ci.yml's Job is live; start fresh skips -2 (nightly's) and takes -3.
    open_sheet(&mut harness, "octo/app workflow .github/workflows/ci.yml");
    assert_eq!(sheet(&harness).status, LaunchStatus::LiveJob);
    press(&mut harness, Key::Char('f'));
    assert_ready(&harness);
    press(&mut harness, Key::Enter);
    let fresh = launch_plans(&harness).last().cloned().expect("a plan");
    assert_eq!(
        (
            fresh.worktree,
            fresh.branch.name.as_str(),
            fresh.session.as_str()
        ),
        (
            worktree(CLONE, "fix-ci_ci-3"),
            "fix-ci_ci-3",
            "app-ci-fix-ci-3"
        )
    );
    assert_eq!(fresh.renamed, None);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
}

// ---- prompts ----

#[test]
fn every_next_step_kind_has_a_default_prompt_that_carries_no_title_or_body() {
    let defaults = Prompts::default();
    let fields = PromptFields {
        repo: "octo/app".to_owned(),
        number: Some(9),
        url: "https://github.com/octo/app/issues/9".to_owned(),
        title: "TITLE-TEXT".to_owned(),
        branch: ISSUE_BRANCH.to_owned(),
        ..PromptFields::default()
    };
    let skills = [
        (NextStep::AddressReview, "/gh-resolve "),
        (NextStep::Implement, "/implement "),
        (NextStep::Triage, "/triage "),
        (NextStep::Review, "/code-review "),
        (NextStep::FixCi, "/diagnosing-bugs "),
        // Milestone 2: Start launches an Implement Job (spec 0002, "Prompts").
        (NextStep::Start, "/implement "),
    ];
    for step in NextStep::ALL {
        let prompt = defaults.render(step, &fields);
        assert!(
            prompt.contains(&fields.url) || prompt.contains("octo/app"),
            "{step:?}: {prompt}"
        );
        assert!(!prompt.contains("TITLE-TEXT"), "{step:?} uses the title");
        match skills.iter().find(|(s, _)| *s == step) {
            Some((_, skill)) => assert!(prompt.starts_with(skill), "{step:?}: {prompt}"),
            None => assert!(!prompt.starts_with('/'), "{step:?}: {prompt}"),
        }
    }
    let branch_fields = PromptFields {
        number: None,
        url: "https://github.com/octo/app/tree/feature/login".to_owned(),
        branch: "feature/login".to_owned(),
        reviewers: Vec::new(),
        slack_channel: None,
        ..fields
    };
    for step in [NextStep::OpenPr, NextStep::PruneOrRevive] {
        let prompt = defaults.render(step, &branch_fields);
        assert!(
            prompt.contains("Branch feature/login in octo/app"),
            "{prompt}"
        );
    }
}

#[test]
fn the_prompts_file_overrides_the_defaults_for_issues_branches_and_runs() {
    let prompts = prompts::parse(
        std::path::Path::new("prompts.toml"),
        "implement = \"/implement {url} on {branch}\"\nopen_pr = \"Open a PR for {branch} in {repo}\"\nfix_ci = \"/diagnosing-bugs {url}\"\n",
    );
    let mut harness = started_with(observation().build(), prompts);
    with_clone(&harness);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature/login", BRANCH_TIP);
    let mut prompt_for = |reference: &str| {
        open_sheet(&mut harness, reference);
        assert_ready(&harness);
        let text = sheet(&harness)
            .prompt
            .as_ref()
            .expect("prompt")
            .text
            .clone();
        press(&mut harness, Key::Esc);
        text
    };
    assert_eq!(
        prompt_for("octo/app#9"),
        format!("/implement https://github.com/octo/app/issues/9 on {ISSUE_BRANCH}")
    );
    assert_eq!(
        prompt_for("octo/app:feature/login"),
        "Open a PR for feature/login in octo/app"
    );
    assert_eq!(
        prompt_for("octo/app workflow .github/workflows/ci.yml"),
        "/diagnosing-bugs https://github.com/octo/app/actions/runs/41"
    );
}

#[test]
fn an_issue_branch_name_from_a_hostile_or_long_title_is_whole_safe_words() {
    let mut harness = started(
        ObservationBuilder::new()
            .owner("octo")
            .repo("octo/app")
            .issue(
                issue("octo/app", 13)
                    .title("../../etc/passwd — Ætla að laga þetta ASAP!!! and a really long tail of words")
                    .assignee(ME)
                    .label("bug"),
            )
            .build(),
    );
    with_clone(&harness);
    open_sheet(&mut harness, "octo/app#13");
    assert_ready(&harness);
    let name = "implement_13-etc-passwd-aetla-ad-laga-thetta-asap-and-a";
    assert_eq!(field(sheet(&harness), "BRANCH").0, name);
    let expected = worktree(CLONE, name);
    assert_eq!(
        field(sheet(&harness), "WORKTREE").0,
        expected.to_str().unwrap()
    );
}
