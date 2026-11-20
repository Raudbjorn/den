//! Seam 2, Land and Mark for release (#37): once both surveys of a round
//! have answered, den reads by number the states of Landing Group members
//! that weren't among the open Pull Requests, merges them into what it shows
//! and caches them; Mark for release follows a member read merged, never a
//! guess. A member that would Merge reads "lands with TEAM-…" in its row and
//! in its Linear Issue's LANDING GROUP section.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Effect, Event, Key, view_model::DetailContent},
    config::Config,
    domain::{
        Coverage, LinearPart, LinearSourceKind, RepoId, Source, SourceKind, SourceState,
        facts::{MemberPullRequestState, PullRequestRef, PullRequestState, ReviewVerdict},
    },
    ports::{LinearSurvey, MemberStatesRead, MemberStatesRequest},
};
use support::{
    build::{ME, ObservationBuilder, linear_issue, linear_viewer, pr},
    harness::{Harness, linear_workflow},
};

const RELEASE: &str = "Ready for Release";

fn api_1() -> PullRequestRef {
    PullRequestRef::new(RepoId::new("acme", "api"), 1)
}

/// acme with a Linear Workflow (release state "Ready for Release"):
///
/// - TEAM-1's only member, api #1, isn't among the open Pull Requests;
/// - TEAM-2's only member, app #2, is open and would Merge.
fn harness() -> Harness {
    let mut workflow = linear_workflow("acme", "acme");
    workflow.release_state = Some(RELEASE.to_owned());
    let config = Config {
        owners: vec!["acme".to_owned()],
        main_clone_root: None,
        main_clones: std::collections::BTreeMap::new(),
        linear: vec![workflow],
    };
    let harness = Harness::new(Ok(config));
    queue_round(&harness);
    harness
}

/// One survey round's answers: GitHub's, then Linear's.
fn queue_round(harness: &Harness) {
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .repo("acme/app")
        .pull_request(
            pr("acme/app", 2)
                .author(ME)
                .passing("ci")
                .approved()
                .review_at_head("alice", ReviewVerdict::Approved),
        )
        .build()));
    let mut coverage = Coverage::default();
    for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
        coverage.record(
            Source::Owner {
                owner: "acme".to_owned(),
                kind: SourceKind::Linear(kind),
            },
            SourceState::Complete,
        );
    }
    harness.linear.push_answer(LinearSurvey {
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues: vec![
                linear_issue("acme", "id-1", "TEAM-1")
                    .attached("acme/api", 1)
                    .build(),
                linear_issue("acme", "id-2", "TEAM-2")
                    .attached("acme/app", 2)
                    .build(),
            ],
            ..LinearPart::default()
        },
        coverage,
    });
}

fn member_states_answer(state: PullRequestState) -> MemberStatesRead {
    let mut coverage = Coverage::default();
    coverage.record(
        Source::Owner {
            owner: "acme".to_owned(),
            kind: SourceKind::MemberStates,
        },
        SourceState::Complete,
    );
    MemberStatesRead {
        states: vec![MemberPullRequestState { pr: api_1(), state }],
        coverage,
    }
}

/// Selects the row whose reference is `reference` and returns its detail.
fn detail_of(harness: &mut Harness, reference: &str) -> DetailContent {
    let index = harness
        .view
        .list
        .rows
        .iter()
        .position(|r| r.reference == reference)
        .unwrap_or_else(|| panic!("{reference} is listed: {:?}", harness.rows()));
    let current = harness.view.list.selected.unwrap_or(0);
    let key = if index >= current { Key::Down } else { Key::Up };
    for _ in 0..index.abs_diff(current) {
        harness.key(key);
    }
    harness.view.detail.content.clone().expect("selected")
}

fn step_label(harness: &Harness, reference: &str) -> String {
    harness
        .rows()
        .into_iter()
        .find(|(_, r, _)| r == reference)
        .unwrap_or_else(|| panic!("{reference} is listed: {:?}", harness.rows()))
        .2
}

/// The read follows both surveys, asks for exactly the member that wasn't
/// open, and a member read merged gives Mark for release, cached with its
/// state. Merged is final: the next round doesn't ask about it again.
#[test]
fn a_member_read_merged_after_the_surveys_gives_mark_for_release() {
    let mut harness = harness();
    harness
        .github
        .push_member_states(member_states_answer(PullRequestState::Merged {
            merged_at: None,
        }));
    harness.start();

    assert_eq!(
        harness.github.member_requests(),
        [MemberStatesRequest {
            pull_requests: vec![api_1()],
        }]
    );
    let order: Vec<&str> = harness
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::SurveyLinear { .. } => Some("linear"),
            Effect::Survey { .. } => Some("github"),
            Effect::ReadMemberStates { .. } => Some("members"),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["linear", "github", "members"]);

    assert_eq!(step_label(&harness, "TEAM-1"), "Mark for release");
    let detail = detail_of(&mut harness, "TEAM-1");
    assert_eq!(detail.next_step, "Mark for release");
    assert_eq!(detail.waiting_on, "you");
    assert!(
        detail
            .also
            .iter()
            .any(|l| l == "evidence: merged acme/api #1"),
        "{:?}",
        detail.also
    );
    let landing = detail.landing.expect("a Landing Group section");
    assert!(
        landing
            .lines
            .iter()
            .any(|l| l.text.contains("acme/api #1") && l.text.contains("merged")),
        "{:?}",
        landing.lines
    );
    let stored = harness.cache.stored();
    let last = stored.last().expect("cached");
    assert_eq!(
        last.observation.member_states,
        [MemberPullRequestState {
            pr: api_1(),
            state: PullRequestState::Merged { merged_at: None },
        }]
    );

    // Next round: api #1 is known merged, so den doesn't ask again.
    queue_round(&harness);
    harness.settle(Event::Key(Key::Char('r')));
    assert_eq!(harness.github.member_requests().len(), 1);
    assert_eq!(step_label(&harness, "TEAM-1"), "Mark for release");
}

/// A member that would Merge reads "lands with TEAM-2" in its row and in its
/// Linear Issue's detail, and the issue reads Land.
#[test]
fn a_ready_member_reads_lands_with_and_its_issue_land() {
    let mut harness = harness();
    harness.start();
    assert_eq!(step_label(&harness, "#2"), "lands with TEAM-2");
    assert_eq!(step_label(&harness, "TEAM-2"), "Land");
    let detail = detail_of(&mut harness, "#2");
    assert_eq!(detail.next_step, "none (lands with TEAM-2)");
    let detail = detail_of(&mut harness, "TEAM-2");
    let landing = detail.landing.expect("a Landing Group section");
    assert!(
        landing
            .lines
            .iter()
            .any(|l| l.text.contains("acme/app #2") && l.text.contains("lands with TEAM-2")),
        "{:?}",
        landing.lines
    );
}

/// A failed read never becomes evidence: TEAM-1 has no Next Step, and its
/// detail says what den couldn't read.
#[test]
fn a_failed_member_read_withholds_mark_for_release_and_says_so() {
    let mut harness = harness();
    // FakeGitHub without a queued answer fails every Owner asked about.
    harness.start();
    assert_eq!(harness.github.member_requests().len(), 1);
    let detail = detail_of(&mut harness, "TEAM-1");
    assert_eq!(detail.next_step, "none");
    assert!(
        detail
            .unknowns
            .iter()
            .any(|u| u.contains("whether acme/api #1 merged or was closed without merging")),
        "{:?}",
        detail.unknowns
    );
    assert!(
        detail
            .coverage
            .iter()
            .any(|c| c.contains("Landing Group member states of acme")),
        "{:?}",
        detail.coverage
    );
}

/// Without Linear Workflows there are no members, so den never asks.
#[test]
fn owners_without_a_linear_workflow_never_read_member_states() {
    let mut harness = Harness::with_owners(&["octo"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("octo")
        .repo("octo/app")
        .pull_request(pr("octo/app", 1).author(ME))
        .build()));
    harness.start();
    assert!(harness.github.member_requests().is_empty());
    assert!(
        !harness
            .effects
            .iter()
            .any(|e| matches!(e, Effect::ReadMemberStates { .. }))
    );
}
