//! Adapter: `Workspace::repo_at` with real git, against temporary repos (#90).
//! A clone and any folder in it read as that clone; a Job worktree reads as
//! its Main Clone, with the same remotes; a folder outside any repo is no
//! work tree. No remote is contacted: adding a remote only writes config.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{handoff::Remote, ports::Workspace, start::GitDir, workspace::GitWorkspace};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Runs git for test setup (not through den), failing the test on error.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=den test",
            "-c",
            "user.email=den@test.invalid",
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
}

/// A clone at `<temp>/app` with one commit and `remotes` (name, URL).
fn clone_with(temp: &Path, remotes: &[(&str, &str)]) -> PathBuf {
    let clone = temp.join("app");
    fs::create_dir_all(&clone).unwrap();
    git(&clone, &["init", "-q"]);
    git(&clone, &["commit", "-q", "--allow-empty", "-m", "seed"]);
    for (name, url) in remotes {
        git(&clone, &["remote", "add", name, url]);
    }
    clone
}

fn work_tree(dir: &Path) -> (PathBuf, Vec<Remote>) {
    match GitWorkspace::new().repo_at(dir).expect("readable") {
        GitDir::WorkTree {
            main_clone,
            remotes,
        } => (main_clone, remotes),
        GitDir::NotAWorkTree => panic!("{} is a work tree", dir.display()),
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    fs::canonicalize(a).unwrap() == fs::canonicalize(b).unwrap()
}

fn names(remotes: &[Remote]) -> Vec<(&str, &str)> {
    let mut names: Vec<(&str, &str)> = remotes
        .iter()
        .map(|r| (r.name.as_str(), r.url.as_str()))
        .collect();
    names.sort_unstable();
    names
}

#[test]
fn a_clone_and_a_folder_in_it_read_as_the_clone_with_its_remotes() {
    let temp = tempfile::tempdir().unwrap();
    let clone = clone_with(
        temp.path(),
        &[
            ("origin", "git@github.com:me/app.git"),
            ("upstream", "https://github.com/acme/app.git"),
        ],
    );
    let nested = clone.join("src/deep");
    fs::create_dir_all(&nested).unwrap();

    for dir in [&clone, &nested] {
        let (main_clone, remotes) = work_tree(dir);
        assert!(same_dir(&main_clone, &clone), "{}", main_clone.display());
        assert_eq!(
            names(&remotes),
            [
                ("origin", "git@github.com:me/app.git"),
                ("upstream", "https://github.com/acme/app.git"),
            ]
        );
    }
}

/// A Job worktree (under the Main Clone's `.claude/worktrees/`) reads as its
/// Main Clone.
#[test]
fn a_job_worktree_reads_as_its_main_clone() {
    let temp = tempfile::tempdir().unwrap();
    let clone = clone_with(temp.path(), &[("origin", "https://github.com/acme/app")]);
    let worktree = clone.join(".claude/worktrees/feat_x");
    git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat_x",
            worktree.to_str().unwrap(),
        ],
    );

    let (main_clone, remotes) = work_tree(&worktree);
    assert!(same_dir(&main_clone, &clone), "{}", main_clone.display());
    assert_eq!(names(&remotes), [("origin", "https://github.com/acme/app")]);
}

/// With `extensions.worktreeConfig`, a Job worktree can carry remotes of its
/// own. They are not the Main Clone's: den reads the Main Clone's remotes, so
/// a worktree-only remote or `origin` never changes which Repo opens.
#[test]
fn a_job_worktree_with_its_own_config_still_reads_the_main_clones_remotes() {
    let temp = tempfile::tempdir().unwrap();
    let clone = clone_with(temp.path(), &[("upstream", "https://github.com/acme/main")]);
    git(&clone, &["config", "extensions.worktreeConfig", "true"]);
    let worktree = clone.join(".claude/worktrees/feat_y");
    git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat_y",
            worktree.to_str().unwrap(),
        ],
    );
    for (key, url) in [
        ("remote.fork.url", "https://github.com/other/fork.git"),
        ("remote.origin.url", "https://github.com/other/origin.git"),
    ] {
        git(&worktree, &["config", "--worktree", key, url]);
    }

    let (main_clone, remotes) = work_tree(&worktree);
    assert!(same_dir(&main_clone, &clone), "{}", main_clone.display());
    assert_eq!(
        names(&remotes),
        [("upstream", "https://github.com/acme/main")]
    );
}

#[test]
fn a_clone_without_remotes_has_none() {
    let temp = tempfile::tempdir().unwrap();
    let clone = clone_with(temp.path(), &[]);
    let (_, remotes) = work_tree(&clone);
    assert!(remotes.is_empty(), "{remotes:?}");
}

#[test]
fn a_folder_outside_any_repo_is_no_work_tree() {
    let temp = tempfile::tempdir().unwrap();
    let plain = temp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert_eq!(
        GitWorkspace::new().repo_at(&plain).expect("readable"),
        GitDir::NotAWorkTree
    );
}

/// Inside a repo's `.git` folder there is no work tree to open.
#[test]
fn the_git_folder_itself_is_no_work_tree() {
    let temp = tempfile::tempdir().unwrap();
    let clone = clone_with(temp.path(), &[("origin", "https://github.com/acme/app")]);
    assert_eq!(
        GitWorkspace::new()
            .repo_at(&clone.join(".git"))
            .expect("readable"),
        GitDir::NotAWorkTree
    );
}

/// A start directory that doesn't exist is an error naming it, not a guess.
#[test]
fn a_missing_directory_is_an_error_naming_it() {
    let temp = tempfile::tempdir().unwrap();
    let gone = temp.path().join("gone");
    let error = GitWorkspace::new().repo_at(&gone).expect_err("missing");
    assert!(
        error.to_string().contains(&gone.display().to_string()),
        "{error}"
    );
}
