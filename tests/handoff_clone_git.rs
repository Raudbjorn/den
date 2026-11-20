//! The git adapter's cloning and new-branch worktrees (#12), on temporary
//! repositories: a bare "remote" stands in for the target Repo, and the clone
//! root is a temp folder, never `~/dev`.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    domain::{NextStep, RepoId, WorkItemId, WorkflowId},
    handoff::{
        self, Base, BaseRef, BranchStart, CloneSource, JobBranch, LaunchOutcome, LaunchPlan,
        MainClone, Remote, Target, WindowCommand, WindowMode, Writability, command,
    },
    jobs::{FileJobStore, JobRecord},
    ports::{JobStore, LaunchError, Launcher, Workspace},
    workspace::GitWorkspace,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

/// Runs git for test setup (not through den), failing the test on error.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=den test",
            "-c",
            "user.email=den@test.invalid",
        ])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A bare remote with one commit on `main`, and an empty clone root.
/// Returns (tempdir, clone root, remote URL, main's commit).
fn hosted_repo() -> (tempfile::TempDir, PathBuf, String, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let base = temp.path();
    let remote = base.join("remote.git");
    let seed = base.join("seed");
    git(
        base,
        &["init", "--bare", "-b", "main", remote.to_str().unwrap()],
    );
    git(base, &["init", "-b", "main", seed.to_str().unwrap()]);
    std::fs::write(seed.join("README"), "hello\n").unwrap();
    git(&seed, &["add", "README"]);
    git(&seed, &["commit", "-m", "first"]);
    let tip = git(&seed, &["rev-parse", "HEAD"]);
    git(
        &seed,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&seed, &["push", "origin", "main"]);
    let root = base.join("dev");
    std::fs::create_dir_all(&root).unwrap();
    (temp, root, remote.to_str().unwrap().to_owned(), tip)
}

#[test]
fn the_git_adapter_asks_a_url_for_a_branch_without_a_clone() {
    let (_temp, _root, url, tip) = hosted_repo();
    let workspace = GitWorkspace::new();
    assert_eq!(
        workspace.url_branch_head(&url, "main").expect("ls-remote"),
        Some(tip)
    );
    assert_eq!(
        workspace.url_branch_head(&url, "gone").expect("ls-remote"),
        None
    );
    assert!(
        workspace
            .url_branch_head("--upload-pack=x", "main")
            .is_err()
    );
    assert!(
        workspace
            .url_branch_head("/nonexistent/den-test.git", "main")
            .is_err()
    );
}

#[test]
fn the_git_adapter_clones_only_into_a_folder_that_does_not_exist() {
    let (_temp, root, url, tip) = hosted_repo();
    let workspace = GitWorkspace::new();

    // An empty folder git itself would happily clone into: den refuses.
    let empty = root.join("empty");
    std::fs::create_dir(&empty).unwrap();
    let error = workspace.clone_repo(&url, &empty).expect_err("refused");
    assert!(error.detail.contains("already exists"), "{error}");
    assert_eq!(std::fs::read_dir(&empty).unwrap().count(), 0);

    assert!(
        workspace
            .clone_repo("-oProxyCommand=x", &root.join("x"))
            .is_err()
    );
    assert!(!root.join("x").exists());

    let into = root.join("app");
    workspace.clone_repo(&url, &into).expect("cloned");
    assert_eq!(git(&into, &["rev-parse", "HEAD"]), tip);
    assert_eq!(
        workspace.remotes(&into).expect("remotes"),
        [Remote {
            name: "origin".to_owned(),
            url: url.clone()
        }]
    );
    // The new clone is found as a Main Clone next time.
    assert_eq!(workspace.find_clones(&root).expect("search"), [into]);
}

/// A launcher that records the window instead of opening one.
#[derive(Default)]
struct RecordingLauncher {
    windows: Mutex<Vec<WindowCommand>>,
}

impl Launcher for RecordingLauncher {
    fn window_mode(&self) -> WindowMode {
        WindowMode::Kitty
    }
    fn claude_available(&self) -> bool {
        true
    }
    fn open_window(&self, window: &WindowCommand) -> Result<(), LaunchError> {
        self.windows.lock().unwrap().push(window.clone());
        Ok(())
    }
    fn copy_to_clipboard(&self, _: &str) -> Result<(), LaunchError> {
        panic!("not used")
    }
}

/// The plan `handoff::plan` makes for a failed `ci.yml` Run when octo/app has
/// no Main Clone yet: clone into `<root>/app`, a new `fix-ci_ci` branch from main.
fn clone_plan(root: &Path, url: &str, tip: &str) -> LaunchPlan {
    let clone = root.join("app");
    let mut plan = LaunchPlan {
        work_item: WorkItemId::Run {
            repo: RepoId::new("octo", "app"),
            workflow: WorkflowId(".github/workflows/ci.yml".to_owned()),
        },
        title: "CI".to_owned(),
        url: "https://github.com/octo/app/actions/runs/41".to_owned(),
        next_step: NextStep::FixCi,
        target: Target {
            repo: RepoId::new("octo", "app"),
            url: url.to_owned(),
            is_fork: false,
        },
        main_clone: MainClone {
            path: clone.clone(),
            source: CloneSource::ToClone {
                url: url.to_owned(),
                root: root.to_owned(),
                taken: Vec::new(),
            },
            writable: Writability::Writable,
        },
        remote: Remote {
            name: "origin".to_owned(),
            url: url.to_owned(),
        },
        base: Base {
            reference: BaseRef::DefaultBranch("main".to_owned()),
            commit: tip.to_owned(),
        },
        branch: JobBranch {
            name: "fix-ci_ci".to_owned(),
            start: BranchStart::Fresh,
        },
        worktree: handoff::worktree_path(&clone, "fix-ci_ci"),
        renamed: None,
        session: "app-ci-fix-ci".to_owned(),
        prompt: "/diagnosing-bugs https://github.com/octo/app/actions/runs/41".to_owned(),
        window: WindowMode::Kitty,
        command: String::new(),
    };
    plan.command = command::display(&plan.window, &plan.window_command());
    plan
}

#[test]
fn executing_a_clone_plan_with_real_git_clones_then_starts_a_new_untracked_branch_on_the_default_branch()
 {
    let (temp, root, url, tip) = hosted_repo();
    let plan = clone_plan(&root, &url, &tip);
    let launcher = RecordingLauncher::default();
    let jobs = FileJobStore::new(temp.path().join("state").join("jobs.json"));
    let report = handoff::execute(
        &plan,
        &GitWorkspace::new(),
        &launcher,
        &jobs,
        "2026-11-20T10:00:00Z".parse().unwrap(),
    );
    assert!(
        matches!(report.outcome, LaunchOutcome::Launched { .. }),
        "{report:#?}"
    );
    assert!(report.done[1].starts_with("cloned "), "{:#?}", report.done);

    let clone = root.join("app");
    let worktree = clone.join(".claude").join("worktrees").join("fix-ci_ci");
    assert_eq!(plan.worktree, worktree);
    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), tip);
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "fix-ci_ci"
    );
    // A new branch tracks nothing: pushing it must not go to main.
    assert_eq!(
        git(
            &clone,
            &["config", "--default", "", "--get", "branch.fix-ci_ci.merge"]
        ),
        ""
    );
    assert_eq!(launcher.windows.lock().unwrap()[0].directory, worktree);
    let recorded: Vec<JobRecord> = jobs.load().expect("records");
    assert_eq!(recorded[0].main_clone, clone);

    // The same plan again: the folder exists now, so den refuses and clones nothing.
    let again = handoff::execute(
        &plan,
        &GitWorkspace::new(),
        &launcher,
        &jobs,
        "2026-11-20T10:05:00Z".parse().unwrap(),
    );
    assert!(
        matches!(again.outcome, LaunchOutcome::Refused(_)),
        "{again:#?}"
    );
    assert!(worktree.join("README").exists());
}

#[test]
fn a_default_branch_that_moved_before_cloning_is_reported_and_nothing_is_cloned() {
    let (_temp, root, url, _tip) = hosted_repo();
    let plan = clone_plan(&root, &url, "0000000000000000000000000000000000000001");
    let report = handoff::execute(
        &plan,
        &GitWorkspace::new(),
        &RecordingLauncher::default(),
        &FileJobStore::new(root.join("unused.json")),
        "2026-11-20T10:00:00Z".parse().unwrap(),
    );
    assert!(
        matches!(&report.outcome, LaunchOutcome::HeadMoved { from, .. } if from.starts_with("0000")),
        "{report:#?}"
    );
    assert!(report.done.is_empty());
    assert!(!root.join("app").exists());
}
