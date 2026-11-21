//! Where den starts (#90; spec 0003, "Screens and navigation", Startup). The
//! first rule that matches wins: `--pick`, then `den OWNER/REPO`, then the git
//! work tree den was started in: a GitHub `origin`, else the only GitHub
//! remote, else no choice with the candidates named. Anywhere else, no choice;
//! a git repo with no GitHub remote says so.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    domain::RepoId,
    handoff::Remote,
    ports::WorkspaceError,
    start::{GitDir, Start, StartArg, resolve},
};
use std::path::PathBuf;

fn remote(name: &str, url: &str) -> Remote {
    Remote {
        name: name.to_owned(),
        url: url.to_owned(),
    }
}

fn clone_with(remotes: &[Remote]) -> Result<GitDir, WorkspaceError> {
    Ok(GitDir::WorkTree {
        main_clone: PathBuf::from("/home/me/dev/app"),
        remotes: remotes.to_vec(),
    })
}

fn opened(start: &Start) -> &RepoId {
    match start {
        Start::Repo(repo) => repo,
        other => panic!("expected a Repo, got {other:?}"),
    }
}

fn notice(start: &Start) -> Option<&str> {
    match start {
        Start::Unchosen { notice } => notice.as_deref(),
        other => panic!("expected no Repo chosen, got {other:?}"),
    }
}

/// Rule 1: `--pick` chooses no Repo even inside a clone, and doesn't look.
#[test]
fn pick_chooses_no_repo_without_reading_the_directory() {
    let start = resolve(&StartArg::Pick, || panic!("--pick must not read git"));
    assert_eq!(notice(&start), None);
}

/// Rule 2: a named Repo opens, without reading the directory.
#[test]
fn a_named_repo_opens_without_reading_the_directory() {
    let start = resolve(&StartArg::Repo(RepoId::new("octo", "app")), || {
        panic!("a named Repo must not read git")
    });
    assert_eq!(opened(&start), &RepoId::new("octo", "app"));
}

/// Rule 3: a GitHub `origin` wins over every other remote, in any URL form,
/// and keeps the URL's spelling.
#[test]
fn a_github_origin_opens_its_repo() {
    for url in [
        "git@github.com:Acme/App.git",
        "https://github.com/Acme/App",
        "ssh://git@github.com/Acme/App.git",
    ] {
        let start = resolve(&StartArg::Auto, || {
            clone_with(&[
                remote("fork", "git@github.com:me/app.git"),
                remote("origin", url),
                remote("upstream", "https://github.com/other/app.git"),
            ])
        });
        let repo = opened(&start);
        assert_eq!(
            (repo.owner.as_str(), repo.name.as_str()),
            ("Acme", "App"),
            "{url}"
        );
    }
}

/// Rule 3: with no GitHub `origin`, the only GitHub remote opens, whether
/// `origin` is missing or points somewhere else. Two remotes naming the same
/// Repo are one candidate.
#[test]
fn without_a_github_origin_the_only_github_remote_opens() {
    let cases: [&[Remote]; 3] = [
        &[remote("upstream", "git@github.com:acme/app.git")],
        &[
            remote("origin", "https://gitlab.com/acme/app.git"),
            remote("hub", "https://github.com/acme/app.git"),
        ],
        &[
            remote("hub", "https://github.com/acme/app.git"),
            remote("hub-ssh", "git@github.com:acme/app.git"),
        ],
    ];
    for remotes in cases {
        let start = resolve(&StartArg::Auto, || clone_with(remotes));
        assert!(
            opened(&start).same(&RepoId::new("acme", "app")),
            "{remotes:?}"
        );
    }
}

/// Rule 3: several GitHub Repos and no GitHub `origin`: den doesn't guess. It
/// names the candidates and how to open one.
#[test]
fn several_github_remotes_without_a_github_origin_name_the_candidates() {
    let cases: [&[Remote]; 2] = [
        &[
            remote("fork", "git@github.com:me/app.git"),
            remote("upstream", "https://github.com/acme/app.git"),
        ],
        &[
            remote("origin", "https://gitlab.com/acme/app.git"),
            remote("fork", "git@github.com:me/app.git"),
            remote("upstream", "https://github.com/acme/app.git"),
        ],
    ];
    for remotes in cases {
        let start = resolve(&StartArg::Auto, || clone_with(remotes));
        let said = notice(&start).expect("a notice");
        // No clone path here (PR #92 review): the header's status is one row,
        // so the notice spends it on the candidates and how to open one.
        for expected in ["me/app", "acme/app", "den OWNER/REPO"] {
            assert!(said.contains(expected), "{expected:?} in {said:?}");
        }
    }
}

/// Rule 4: a git repo without a GitHub remote says so; a directory that isn't
/// a git work tree chooses quietly.
#[test]
fn no_github_remote_says_so_and_no_repo_is_quiet() {
    let local = resolve(&StartArg::Auto, || {
        clone_with(&[remote("origin", "/srv/git/app.git")])
    });
    let said = notice(&local).expect("a notice");
    assert!(said.contains("no GitHub remote"), "{said}");
    assert!(said.contains("/home/me/dev/app"), "{said}");

    let none = resolve(&StartArg::Auto, || clone_with(&[]));
    assert!(notice(&none).is_some_and(|n| n.contains("no GitHub remote")));

    let outside = resolve(&StartArg::Auto, || Ok(GitDir::NotAWorkTree));
    assert_eq!(notice(&outside), None);
}

/// A directory den couldn't read chooses no Repo and says why, never failing
/// the start.
#[test]
fn an_unreadable_directory_chooses_no_repo_and_says_why() {
    let start = resolve(&StartArg::Auto, || {
        Err(WorkspaceError {
            operation: "read the git work tree at /x".to_owned(),
            detail: "timed out after 30 s".to_owned(),
        })
    });
    let said = notice(&start).expect("a notice");
    assert!(said.contains("timed out after 30 s"), "{said}");
}

/// Only github.com counts: a GitHub Enterprise remote is not a GitHub Repo.
#[test]
fn only_github_com_remotes_are_github_repos() {
    let start = resolve(&StartArg::Auto, || {
        clone_with(&[remote("origin", "git@github.example.com:acme/app.git")])
    });
    assert!(notice(&start).is_some_and(|n| n.contains("no GitHub remote")));
}
