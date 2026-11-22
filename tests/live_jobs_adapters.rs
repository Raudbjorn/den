//! Thin check of the Live Job worktree states (#14) against real git: a temp
//! Main Clone with Job worktrees added by git itself, then locked, removed by
//! hand (as a user might), or never created. den's check reads only.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    domain::{NextStep, RepoId, WorkItemId},
    handoff::{WorktreeState, live},
    jobs::{JobRecord, JobSubject},
    workspace::GitWorkspace,
};
use jiff::Timestamp;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn job(clone: &Path, number: u64, name: &str) -> JobRecord {
    JobRecord {
        subject: JobSubject::GitHub(WorkItemId::PullRequest {
            repo: RepoId::new("octo", "app"),
            number,
        }),
        next_step: NextStep::AddressReview,
        target: RepoId::new("octo", "app"),
        main_clone: clone.to_owned(),
        worktree: clone.join(".claude").join("worktrees").join(name),
        branch: name.to_owned(),
        session: format!("app-pr{number}-address-review"),
        launched_at: Timestamp::UNIX_EPOCH,
        permission_mode: None,
    }
}

#[test]
fn real_git_worktrees_are_present_missing_or_unknown() {
    let temp = tempfile::tempdir().unwrap();
    // git prints real paths; den's records hold the path den computed.
    let clone: PathBuf = fs::canonicalize(temp.path()).unwrap().join("app");
    fs::create_dir(&clone).unwrap();
    git(&clone, &["init", "-q"]);
    git(&clone, &["commit", "-q", "--allow-empty", "-m", "start"]);
    let add = |name: &str| {
        let path = clone.join(".claude/worktrees").join(name);
        git(
            &clone,
            &["worktree", "add", "-q", "-b", name, path.to_str().unwrap()],
        );
        path
    };
    let present = job(&clone, 1, "present");
    add("present");
    let locked = job(&clone, 2, "locked");
    git(
        &clone,
        &["worktree", "lock", add("locked").to_str().unwrap()],
    );
    let removed = job(&clone, 3, "removed");
    fs::remove_dir_all(add("removed")).unwrap();
    let never = job(&clone, 4, "never");
    let unregistered = job(&clone, 5, "unregistered");
    fs::create_dir_all(&unregistered.worktree).unwrap();

    let records = [
        present.clone(),
        locked.clone(),
        removed.clone(),
        never.clone(),
        unregistered.clone(),
    ];
    let check = live::check_jobs(&records, &GitWorkspace::new(), Duration::from_secs(60));
    let state_of = |job: &JobRecord| {
        check
            .checks
            .iter()
            .find(|c| c.job == *job)
            .map(|c| c.state.clone())
            .expect("checked")
    };

    assert_eq!(state_of(&present), WorktreeState::Present);
    assert_eq!(state_of(&never), WorktreeState::Missing);
    for job in [&locked, &removed, &unregistered] {
        assert!(
            matches!(state_of(job), WorktreeState::Unknown(_)),
            "{}: {:?}",
            job.branch,
            state_of(job)
        );
    }
    // Reading changed nothing: the hand-removed worktree is still registered
    // (den never prunes), and the locked one is still there.
    assert!(locked.worktree.exists());
    let listed = git(&clone, &["worktree", "list", "--porcelain"]);
    assert!(
        listed.contains(&format!("worktree {}", removed.worktree.display())),
        "{listed}"
    );
    assert_eq!(check.not_checked, 0);
}

/// A Main Clone configured through a symlink (review on #27): den records the
/// Job under the alias, git lists the worktree under the real path. Both name
/// the same folder, so the Job is a Live Job, not unknown.
#[cfg(unix)]
#[test]
fn a_main_clone_reached_through_a_symlink_still_has_live_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let real: PathBuf = fs::canonicalize(temp.path()).unwrap().join("real");
    fs::create_dir(&real).unwrap();
    git(&real, &["init", "-q"]);
    git(&real, &["commit", "-q", "--allow-empty", "-m", "start"]);
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();

    let feature = job(&alias, 7, "feature");
    git(
        &alias,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            feature.worktree.to_str().unwrap(),
        ],
    );
    // git itself names it by the real path.
    let listed = git(&alias, &["worktree", "list", "--porcelain"]);
    assert!(
        listed.contains(&format!(
            "worktree {}",
            real.join(".claude/worktrees/feature").display()
        )),
        "{listed}"
    );
    // A Job whose folder is gone is still told apart from a present one.
    let gone = job(&alias, 8, "gone");

    let check = live::check_jobs(
        &[feature.clone(), gone.clone()],
        &GitWorkspace::new(),
        Duration::from_secs(60),
    );
    let state_of = |job: &JobRecord| {
        check
            .checks
            .iter()
            .find(|c| c.job == *job)
            .map(|c| c.state.clone())
            .expect("checked")
    };
    assert_eq!(state_of(&feature), WorktreeState::Present);
    assert_eq!(state_of(&gone), WorktreeState::Missing);

    // The port answers in den's names: the clone's own checkout and the Job's
    // worktree both under the alias, exactly (no trailing separator).
    let registered = den::ports::Workspace::worktrees(&GitWorkspace::new(), &alias).unwrap();
    let paths: Vec<String> = registered
        .iter()
        .map(|w| w.path.display().to_string())
        .collect();
    assert_eq!(
        paths,
        [
            alias.display().to_string(),
            feature.worktree.display().to_string()
        ]
    );
}

/// #80: the real process read. A child whose command line is
/// `claude -n <session> …` (`tail` under the name `claude`, so it waits
/// without doing anything) reads as running; once it has exited, idle.
#[cfg(target_os = "linux")]
#[test]
fn a_claude_process_naming_the_session_reads_as_running_then_idle_once_it_exits() {
    use den::{handoff::SessionActivity, ports::Workspace};
    use std::os::unix::process::CommandExt;

    // A number, as `tail -n` needs, unique to this test run.
    let session = (4_000_000_000u64 + u64::from(std::process::id())).to_string();
    let read = || {
        GitWorkspace::new()
            .user_processes()
            .map(|processes| live::running_sessions(&processes))
            .map_err(|e| e.to_string())
    };
    assert_eq!(live::activity(&session, &read()), SessionActivity::Idle);

    let mut child = Command::new("tail")
        .arg0("claude")
        .args(["-n", &session, "-f", "/dev/null"])
        .spawn()
        .expect("tail runs");
    // The kernel shows the command line once the child has exec'd.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut seen = live::activity(&session, &read());
    while seen != SessionActivity::Running && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        seen = live::activity(&session, &read());
    }
    assert_eq!(seen, SessionActivity::Running);

    child.kill().expect("kill");
    child.wait().expect("reaped");
    assert_eq!(live::activity(&session, &read()), SessionActivity::Idle);
}

/// #68, stories 67-69: a bounded read must not turn a hidden session flag
/// into evidence that a Live Job is idle.
#[cfg(target_os = "linux")]
#[test]
fn command_line_overflow_is_unknown_even_one_byte_past_the_limit() {
    use den::{
        handoff::SessionActivity,
        processes::{MAX_CMDLINE_BYTES, read_user_processes},
    };

    let proc_root = tempfile::tempdir().unwrap();
    fs::create_dir(proc_root.path().join("self")).unwrap();
    fs::create_dir(proc_root.path().join("42")).unwrap();
    let path = proc_root.path().join("42/cmdline");
    let read = || {
        read_user_processes(proc_root.path()).map(|processes| live::running_sessions(&processes))
    };

    let tail = b"\0-n\0at-limit\0";
    let mut complete = b"claude\0".to_vec();
    complete.resize(MAX_CMDLINE_BYTES as usize - tail.len(), b'x');
    complete.extend_from_slice(tail);
    fs::write(&path, &complete).unwrap();
    assert_eq!(
        live::activity("at-limit", &read()),
        SessionActivity::Running
    );

    for suffix in [b"x".as_slice(), b"-n\0past-limit\0"] {
        let mut oversized = complete.clone();
        oversized.extend_from_slice(suffix);
        fs::write(&path, oversized).unwrap();
        let result = read();
        assert!(
            matches!(
                live::activity("past-limit", &result),
                SessionActivity::Unknown(_)
            ),
            "an incomplete process read must not report idle: {result:?}"
        );
        let error = result.unwrap_err();
        assert!(error.contains("42/cmdline"), "{error}");
        assert!(error.contains(&MAX_CMDLINE_BYTES.to_string()), "{error}");
        assert!(!error.contains("at-limit") && !error.contains("past-limit"));
    }
}
