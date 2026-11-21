//! Seam 2, where den starts (#90): the start rules' answer decides the Scope den
//! opens in. A watched Owner's Repo opens in its Repo Scope; a Repo outside the
//! watched Owners can't be surveyed yet (the Repo Survey comes later in
//! milestone 3, Plan 2), so den opens All Owners and says so; no Repo chosen
//! opens All Owners, with the start rules' notice when there is one.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Event, Key},
    domain::{Observation, RepoId},
    handoff::Remote,
    start::{GitDir, Start, StartArg, resolve},
    ui::{Theme, frame},
};
use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};
use std::path::PathBuf;
use support::{
    build::{ME, ObservationBuilder, now, pr},
    harness::Harness,
};

fn two_owners() -> Observation {
    ObservationBuilder::new()
        .owner("octo")
        .owner("beta")
        .repo("octo/app")
        .repo("beta/quiet")
        .pull_request(pr("octo/app", 1).draft().author(ME).title("mine"))
        .pull_request(pr("beta/quiet", 5).draft().author(ME).title("elsewhere"))
        .build()
}

fn started_in(start: Start) -> Harness {
    let mut harness = Harness::with_owners(&["octo", "beta"]).with_start(start);
    harness.answer(Ok(two_owners()));
    harness.start();
    harness
}

fn references(harness: &Harness) -> Vec<String> {
    harness
        .rows()
        .into_iter()
        .map(|(repo, reference, _)| format!("{repo}{reference}"))
        .collect()
}

/// Acceptance: a watched Owner's Repo opens in its Repo Scope, in any casing,
/// and stays there across a refresh.
#[test]
fn a_watched_repo_opens_in_its_repo_scope() {
    for spelled in [RepoId::new("octo", "app"), RepoId::new("OCTO", "App")] {
        let mut harness = started_in(Start::Repo(spelled.clone()));
        assert!(
            harness
                .view
                .header
                .scope
                .to_lowercase()
                .starts_with("repo · octo/app"),
            "{spelled}: {}",
            harness.view.header.scope
        );
        assert_eq!(references(&harness), ["octo/app#1"], "{spelled}");

        harness.answer(Ok(two_owners()));
        harness.settle(Event::Started { now: now() });
        assert_eq!(references(&harness), ["octo/app#1"], "after a refresh");
    }
}

/// Acceptance: a Repo outside the watched Owners opens All Owners, with a
/// notice naming it.
#[test]
fn a_repo_outside_the_watched_owners_opens_all_owners_and_says_so() {
    let harness = started_in(Start::Repo(RepoId::new("stranger", "lib")));
    assert!(
        harness.view.header.scope.starts_with("All Owners"),
        "{}",
        harness.view.header.scope
    );
    assert_eq!(references(&harness).len(), 2);
    let status = &harness.view.header.status;
    assert!(status.contains("stranger/lib"), "{status}");
    assert!(status.contains("not a watched Owner"), "{status}");
}

/// Acceptance: no Repo chosen opens All Owners, with the start rules' notice
/// when they gave one, and without one when they didn't.
#[test]
fn no_repo_chosen_opens_all_owners_with_any_notice() {
    let noticed = started_in(Start::Unchosen {
        notice: Some("/home/me/dev/app has no GitHub remote".to_owned()),
    });
    assert!(noticed.view.header.scope.starts_with("All Owners"));
    assert!(
        noticed
            .view
            .header
            .status
            .contains("/home/me/dev/app has no GitHub remote"),
        "{}",
        noticed.view.header.status
    );

    let quiet = started_in(Start::Unchosen { notice: None });
    let plain = {
        let mut harness = Harness::with_owners(&["octo", "beta"]);
        harness.answer(Ok(two_owners()));
        harness.start();
        harness
    };
    assert_eq!(quiet.view.header, plain.view.header, "no notice, no change");
}

/// The start notice is about where den opened: once the user chooses another
/// Scope, it goes.
#[test]
fn the_start_notice_goes_once_the_scope_changes() {
    let mut harness = started_in(Start::Repo(RepoId::new("stranger", "lib")));
    assert!(harness.view.header.status.contains("stranger/lib"));
    harness.key(Key::Right);
    assert!(
        !harness.view.header.status.contains("stranger/lib"),
        "{}",
        harness.view.header.status
    );
}

/// The header's status is one row: the start notice leads it, so what den
/// found (the candidates, and how to open one) is on screen from the minimum
/// size up, after the clone path a long home directory might add.
#[test]
fn the_candidates_are_on_screen_at_every_supported_width() {
    let start = resolve(&StartArg::Auto, || {
        Ok(GitDir::WorkTree {
            main_clone: PathBuf::from("/home/somebody-with-a-long-name/dev/work/clients/app"),
            remotes: vec![
                Remote {
                    name: "fork".to_owned(),
                    url: "git@github.com:me/app.git".to_owned(),
                },
                Remote {
                    name: "upstream".to_owned(),
                    url: "https://github.com/acme/app.git".to_owned(),
                },
            ],
        })
    });
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::TrueColor,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    for (cols, rows) in [(80, 24), (120, 40), (160, 45)] {
        let mut harness = Harness::with_owners(&["octo", "beta"]).with_start(start.clone());
        harness.send(Event::Resized { cols, rows });
        harness.answer(Ok(two_owners()));
        harness.start();
        let text =
            frame::buffer_text(&frame::render(&theme, &harness.view, cols, rows).expect("renders"));
        for shown in ["me/app", "acme/app"] {
            assert!(
                text.contains(shown),
                "{cols}x{rows}: {shown:?} not on screen:\n{text}"
            );
        }
    }
}
