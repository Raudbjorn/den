//! Thin checks on the real launch adapters (spec, Testing Decisions: "the real
//! git and kitty adapters beyond thin smoke checks" are not tested). The git
//! adapter runs against temporary repositories made here (a bare "remote" and
//! a clone of it); kitty and the clipboard are checked only by the command they
//! would run; nothing here opens a window or touches the clipboard.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    domain::{NextStep, RepoId, WorkItemId},
    handoff::{
        self, Base, BaseRef, BranchStart, CloneSource, JobBranch, LaunchOutcome, LaunchPlan,
        MainClone, PathState, Remote, SessionStart, Target, WindowCommand, WindowMode, Writability,
        clone::{RepoUrl, has_numeric_suffix, same_repo},
        command, names,
    },
    jobs::{FileJobStore, JobRecord},
    launcher::{LaunchEnvironment, clipboard_invocation, window_mode},
    ports::{JobStore, LaunchError, Launcher, Workspace, WorktreeAdd, WorktreeStart},
    process::{Invocation, OutputMode},
    workspace::GitWorkspace,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

// ---- kitty and the clipboard: the commands they would run ----

fn window() -> WindowCommand {
    WindowCommand {
        directory: "/home/me/dev/app/.claude/worktrees/feature-7".into(),
        session: "app-pr7-address-review".to_owned(),
        prompt: "/gh-resolve https://github.com/octo/app/pull/7 — it's fine".to_owned(),
        start: SessionStart::New,
    }
}

#[test]
fn kitty_opens_a_new_os_window_in_the_worktree_running_claude_without_bypass_flags() {
    let invocation = command::kitty_invocation(&window());
    let args: Vec<String> = invocation
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(invocation.program, "kitty");
    assert_eq!(
        args,
        [
            "--detach",
            "--hold",
            "--directory",
            "/home/me/dev/app/.claude/worktrees/feature-7",
            "claude",
            "-n",
            "app-pr7-address-review",
            "/gh-resolve https://github.com/octo/app/pull/7 — it's fine",
        ]
    );
    assert!(
        !args
            .iter()
            .any(|a| a.contains("dangerously") || a.contains("permission"))
    );
    assert_eq!(
        command::display(&WindowMode::Kitty, &window()),
        "kitty --detach --hold --directory /home/me/dev/app/.claude/worktrees/feature-7 claude -n app-pr7-address-review '/gh-resolve https://github.com/octo/app/pull/7 — it'\\''s fine'"
    );
}

fn environment(in_kitty: bool, wayland: bool, x11: bool) -> LaunchEnvironment {
    LaunchEnvironment {
        in_kitty,
        wayland,
        x11,
        path: Vec::new(),
    }
}

#[test]
fn a_window_opens_only_inside_kitty_with_kitty_on_path() {
    assert_eq!(
        window_mode(&environment(true, true, false), true),
        WindowMode::Kitty
    );
    assert!(matches!(
        window_mode(&environment(true, true, false), false),
        WindowMode::Clipboard { why } if why.contains("not on PATH")
    ));
    assert!(matches!(
        window_mode(&environment(false, true, false), true),
        WindowMode::Clipboard { why } if why == "den is not running in kitty"
    ));
    let from_env = LaunchEnvironment::from_env(|name| match name {
        "KITTY_WINDOW_ID" => Some("18".into()),
        "WAYLAND_DISPLAY" => Some("wayland-1".into()),
        "DISPLAY" => Some("".into()),
        _ => None,
    });
    assert_eq!(from_env, environment(true, true, false));
}

#[test]
fn the_clipboard_uses_wl_copy_on_wayland_then_xclip_or_xsel_and_says_what_to_install() {
    let all = |_: &str| true;
    let wl = clipboard_invocation(&environment(false, true, true), all).expect("a tool");
    assert_eq!(wl, Invocation::new("wl-copy").discard_output());
    assert_eq!(wl.output, OutputMode::Discard);

    let xclip = clipboard_invocation(&environment(false, false, true), all).expect("a tool");
    assert_eq!(
        xclip,
        Invocation::new("xclip")
            .args(["-selection", "clipboard"])
            .discard_output()
    );
    let xsel =
        clipboard_invocation(&environment(false, false, true), |p| p == "xsel").expect("a tool");
    assert_eq!(xsel.program, "xsel");

    assert_eq!(
        clipboard_invocation(&environment(false, true, false), |_| false),
        Err("no clipboard tool: install wl-clipboard (wl-copy)".to_owned())
    );
    assert!(clipboard_invocation(&environment(false, false, false), all).is_err());
    assert_eq!(
        command::clipboard_text(&window()),
        "cd /home/me/dev/app/.claude/worktrees/feature-7 && claude -n app-pr7-address-review '/gh-resolve https://github.com/octo/app/pull/7 — it'\\''s fine'"
    );
}

// ---- pure naming and matching ----

#[test]
fn remote_urls_match_a_repo_in_every_form_git_accepts_and_only_that_repo() {
    let repo = "https://github.com/Octo/App";
    for url in [
        "https://github.com/octo/app.git",
        "https://user@github.com/octo/app",
        "git@github.com:Octo/app.git",
        "ssh://git@github.com:22/octo/app.git",
        "git://github.com/octo/app",
    ] {
        assert!(same_repo(url, repo), "{url}");
    }
    for url in [
        "https://github.com/octo/app-2",
        "https://github.com/other/app",
        "https://gitlab.com/octo/app",
        "/home/me/dev/app",
        "../app",
    ] {
        assert!(!same_repo(url, repo), "{url}");
    }
    // Not a hosted Repo's URL: only the same location matches.
    assert!(same_repo("/srv/git/app.git", "/srv/git/app/"));
    assert!(!same_repo("/srv/git/app.git", "/srv/git/app-2.git"));
    assert_eq!(
        RepoUrl::parse("git@github.com:octo/app.git"),
        Some(RepoUrl {
            host: "github.com".to_owned(),
            owner: "octo".to_owned(),
            name: "app".to_owned()
        })
    );
    assert!(has_numeric_suffix("app-2") && has_numeric_suffix("app2"));
    assert!(!has_numeric_suffix("app") && !has_numeric_suffix("app-two"));
}

#[test]
fn worktree_and_session_names_from_branch_names_stay_inside_the_worktrees_folder() {
    assert_eq!(
        names::worktree_name("feat/11-launch-pr"),
        "feat-11-launch-pr"
    );
    assert_eq!(names::worktree_name("../../etc"), "etc");
    assert_eq!(names::worktree_name("-rf"), "rf");
    assert_eq!(names::worktree_name("a..b"), "a-b");
    assert_eq!(names::worktree_name("..."), "job");
    assert_eq!(names::worktree_name("ünïcode branch"), "n-code-branch");
    assert_eq!(
        names::pr_session_name(&RepoId::new("octo", "app"), 7, NextStep::FixCi),
        "app-pr7-fix-ci"
    );
}

// ---- the git adapter, on temporary repositories ----

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

/// A bare "remote" with `main` and a PR head branch `feature/x`, a clone of it
/// under `<root>/dev/app` with the remote named `upstream`, and a stray copy
/// `<root>/dev/app-2`. Returns (tempdir, root/dev, clone, remote URL, head commit).
fn temp_repos() -> (tempfile::TempDir, PathBuf, PathBuf, String, String) {
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
    git(&seed, &["checkout", "-b", "feature/x"]);
    std::fs::write(seed.join("README"), "hello, PR\n").unwrap();
    git(&seed, &["commit", "-am", "the PR"]);
    let head = git(&seed, &["rev-parse", "HEAD"]);
    git(
        &seed,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&seed, &["push", "origin", "main", "feature/x"]);

    let dev = base.join("dev");
    std::fs::create_dir_all(dev.join("nested")).unwrap();
    let clone = dev.join("app");
    let url = remote.to_str().unwrap().to_owned();
    git(
        base,
        &["clone", "-o", "upstream", &url, clone.to_str().unwrap()],
    );
    git(base, &["clone", &url, dev.join("app-2").to_str().unwrap()]);
    git(
        base,
        &[
            "clone",
            &url,
            dev.join("nested").join("deep").to_str().unwrap(),
        ],
    );
    (temp, dev, clone, url, head)
}

#[test]
fn the_git_adapter_finds_clones_reads_remotes_and_resolves_refs() {
    let (_temp, dev, clone, url, head) = temp_repos();
    let workspace = GitWorkspace::new();

    let found = workspace.find_clones(&dev).expect("search");
    assert_eq!(
        found,
        [
            dev.join("app"),
            dev.join("app-2"),
            dev.join("nested").join("deep")
        ]
    );
    assert!(workspace.find_clones(&dev.join("missing")).is_err());

    assert_eq!(
        workspace.remotes(&clone).expect("remotes"),
        [Remote {
            name: "upstream".to_owned(),
            url: url.clone()
        }]
    );
    assert_eq!(
        workspace
            .remote_branch_head(&clone, "upstream", "feature/x")
            .expect("ls-remote"),
        Some(head.clone())
    );
    assert_eq!(
        workspace
            .remote_branch_head(&clone, "upstream", "gone")
            .expect("ls-remote"),
        None
    );
    assert!(
        workspace
            .remote_branch_head(&clone, "--upload-pack=x", "main")
            .is_err()
    );
    assert_eq!(
        workspace
            .resolve(&clone, "refs/heads/feature/x")
            .expect("resolve"),
        None
    );
    assert!(matches!(
        workspace.path_state(&clone),
        PathState::Exists {
            writable: Writability::Writable
        }
    ));
    assert_eq!(
        workspace.path_state(&clone.join("nope")),
        PathState::Missing
    );
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

fn plan_for(clone: &Path, url: &str, head: &str) -> LaunchPlan {
    let worktree = handoff::worktree_path(clone, &names::worktree_name("feature/x"));
    let mut plan = LaunchPlan {
        work_item: WorkItemId::PullRequest {
            repo: RepoId::new("octo", "app"),
            number: 7,
        },
        title: "The PR".to_owned(),
        url: "https://github.com/octo/app/pull/7".to_owned(),
        next_step: NextStep::AddressReview,
        target: Target {
            repo: RepoId::new("octo", "app"),
            url: url.to_owned(),
            is_fork: false,
            upstream_of: None,
            default_branch: Some("main".to_owned()),
        },
        main_clone: MainClone {
            path: clone.to_owned(),
            source: CloneSource::Configured,
            writable: Writability::Writable,
        },
        remote: Remote {
            name: "upstream".to_owned(),
            url: url.to_owned(),
        },
        base: Base {
            reference: BaseRef::HeadBranch("feature/x".to_owned()),
            commit: head.to_owned(),
        },
        branch: JobBranch {
            name: "feature/x".to_owned(),
            start: BranchStart::New {
                tracking: "upstream/feature/x".to_owned(),
            },
        },
        worktree,
        renamed: None,
        session: "app-pr7-address-review".to_owned(),
        prompt: "/gh-resolve https://github.com/octo/app/pull/7".to_owned(),
        window: WindowMode::Kitty,
        command: String::new(),
    };
    plan.command = command::display(&plan.window, &plan.window_command());
    plan
}

#[test]
fn executing_a_plan_with_real_git_puts_the_worktree_on_the_pr_head_tracking_the_target_remote() {
    let (temp, _dev, clone, url, head) = temp_repos();
    // The "target Repo" is the local bare repository: its path is its URL.
    let plan = plan_for(&clone, &url, &head);
    let workspace = GitWorkspace::new();
    let launcher = RecordingLauncher::default();
    let jobs = FileJobStore::new(temp.path().join("state").join("jobs.json"));
    let report = handoff::execute(
        &plan,
        &workspace,
        &launcher,
        &jobs,
        "2026-11-20T10:00:00Z".parse().unwrap(),
    );

    assert!(
        matches!(report.outcome, LaunchOutcome::Launched { .. }),
        "{report:#?}"
    );
    let worktree = clone.join(".claude").join("worktrees").join("feature-x");
    assert_eq!(plan.worktree, worktree);
    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/x"
    );
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "upstream/feature/x"
    );
    assert_eq!(launcher.windows.lock().unwrap()[0].directory, worktree);
    let recorded: Vec<JobRecord> = jobs.load().expect("records");
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].worktree, worktree);

    // Launching again finds the worktree in place and refuses; nothing is removed.
    let again = handoff::execute(
        &plan,
        &workspace,
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
fn a_moved_head_on_the_real_remote_is_reported_before_anything_changes() {
    let (_temp, _dev, clone, url, _head) = temp_repos();
    let plan = plan_for(&clone, &url, "0000000000000000000000000000000000000001");

    let jobs = FileJobStore::new(clone.join("unused.json"));
    let report = handoff::execute(
        &plan,
        &GitWorkspace::new(),
        &RecordingLauncher::default(),
        &jobs,
        "2026-11-20T10:00:00Z".parse().unwrap(),
    );
    assert!(
        matches!(&report.outcome, LaunchOutcome::HeadMoved { from, .. } if from.starts_with("0000")),
        "{report:#?}"
    );
    assert!(report.done.is_empty());
    assert!(!plan.worktree.exists());
    assert_eq!(
        GitWorkspace::new()
            .resolve(&clone, "refs/heads/feature/x")
            .expect("resolve"),
        None
    );
}

#[test]
fn the_git_adapter_adds_a_worktree_for_an_existing_local_branch() {
    let (_temp, _dev, clone, _url, head) = temp_repos();
    let workspace = GitWorkspace::new();
    workspace
        .fetch(
            &clone,
            "upstream",
            "+refs/heads/feature/x:refs/remotes/upstream/feature/x",
        )
        .expect("fetch");
    git(&clone, &["branch", "feature/x", "upstream/feature/x"]);
    let path = clone.join(".claude/worktrees/feature-x");
    workspace
        .add_worktree(
            &clone,
            &WorktreeAdd {
                path: path.clone(),
                branch: "feature/x".to_owned(),
                start: WorktreeStart::ExistingBranch,
            },
        )
        .expect("worktree add");
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), head);
    let registered = workspace.worktrees(&clone).expect("worktree list");
    assert_eq!(registered.len(), 2, "{registered:?}");
    assert_eq!(registered[0].branch.as_deref(), Some("main"));
    assert_eq!(registered[1].branch.as_deref(), Some("feature/x"));
    assert_eq!(
        registered[1].path.canonicalize().unwrap(),
        path.canonicalize().unwrap()
    );
    assert!(!registered[1].locked && !registered[1].prunable);
    // A second add on the same path fails with git's own message, nothing removed.
    let error = workspace
        .add_worktree(
            &clone,
            &WorktreeAdd {
                path: path.clone(),
                branch: "feature/x".to_owned(),
                start: WorktreeStart::ExistingBranch,
            },
        )
        .expect_err("already there");
    assert!(error.operation.starts_with("git worktree add"), "{error}");
    assert!(path.join("README").exists());
}

// ---- Job records on disk ----

#[test]
fn job_records_are_appended_and_an_unreadable_file_is_never_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state").join("jobs.json");
    let store = FileJobStore::new(path.clone());
    assert_eq!(store.load().expect("no file yet"), []);
    let job = |n: u64| JobRecord {
        work_item: WorkItemId::PullRequest {
            repo: RepoId::new("octo", "app"),
            number: n,
        },
        next_step: NextStep::Review,
        target: RepoId::new("octo", "app"),
        main_clone: "/c".into(),
        worktree: format!("/c/.claude/worktrees/{n}").into(),
        branch: format!("b{n}"),
        session: format!("s{n}"),
        launched_at: "2026-11-20T10:00:00Z".parse().unwrap(),
    };
    store.record(&job(1)).unwrap();
    store.record(&job(2)).unwrap();
    assert_eq!(store.load().unwrap(), [job(1), job(2)]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    std::fs::write(&path, "{ not json").unwrap();
    assert!(store.load().is_err());
    assert!(store.record(&job(3)).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
}

#[test]
fn a_cross_repo_head_is_fetched_by_its_pr_ref_and_the_branch_tracks_nothing() {
    // #13: the head branch lives in another Repo, so the target's remote has no
    // branch of that name, only GitHub's `refs/pull/<n>/head`.
    let (temp, _dev, clone, url, head) = temp_repos();
    let remote = temp.path().join("remote.git");
    git(&remote, &["update-ref", "refs/pull/7/head", &head]);
    git(&remote, &["update-ref", "-d", "refs/heads/feature/x"]);
    let workspace = GitWorkspace::new();
    assert_eq!(
        workspace
            .remote_pull_head(&clone, "upstream", 7)
            .expect("ls-remote"),
        Some(head.clone())
    );
    assert_eq!(
        workspace
            .remote_pull_head(&clone, "upstream", 8)
            .expect("ls-remote"),
        None
    );

    let mut plan = plan_for(&clone, &url, &head);
    plan.base.reference = BaseRef::PullHead { number: 7 };
    plan.branch.start = BranchStart::NewUntracked;
    let launcher = RecordingLauncher::default();
    let jobs = FileJobStore::new(temp.path().join("state").join("jobs.json"));
    let report = handoff::execute(
        &plan,
        &workspace,
        &launcher,
        &jobs,
        "2026-11-20T10:00:00Z".parse().unwrap(),
    );
    assert!(
        matches!(report.outcome, LaunchOutcome::Launched { .. }),
        "{report:#?}"
    );
    assert_eq!(
        git(&clone, &["rev-parse", "refs/remotes/upstream/pr/7"]),
        head
    );
    assert_eq!(git(&plan.worktree, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&plan.worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/x"
    );
    let tracking = Command::new("git")
        .arg("-C")
        .arg(&plan.worktree)
        .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
        .output()
        .expect("git runs");
    assert!(
        !tracking.status.success(),
        "the branch tracks nothing: {}",
        String::from_utf8_lossy(&tracking.stdout)
    );
}
