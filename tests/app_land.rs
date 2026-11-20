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
    app::{
        Effect, Event, Key,
        view_model::{DetailContent, LaunchStatus, Overlay},
    },
    config::Config,
    domain::{
        Coverage, LinearPart, LinearSourceKind, RepoId, Source, SourceKind, SourceState,
        facts::{
            LinearIssue, MemberPullRequestState, PullRequestRef, PullRequestState, ReviewVerdict,
        },
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
    harness_with_issues(default_issues())
}

/// `harness()`, but its first round's Linear Issues are `issues`.
fn harness_with_issues(issues: Vec<LinearIssue>) -> Harness {
    let mut workflow = linear_workflow("acme", "acme");
    workflow.release_state = Some(RELEASE.to_owned());
    let config = Config {
        owners: vec!["acme".to_owned()],
        main_clone_root: None,
        main_clones: std::collections::BTreeMap::new(),
        linear: vec![workflow],
    };
    let harness = Harness::new(Ok(config));
    queue_round_with(&harness, issues);
    harness
}

/// One survey round's answers: GitHub's, then Linear's.
fn queue_round(harness: &Harness) {
    queue_round_with(harness, default_issues());
}

/// TEAM-1 with api #1 (not among the open Pull Requests), TEAM-2 with app #2.
fn default_issues() -> Vec<LinearIssue> {
    vec![
        linear_issue("acme", "id-1", "TEAM-1")
            .attached("acme/api", 1)
            .build(),
        linear_issue("acme", "id-2", "TEAM-2")
            .attached("acme/app", 2)
            .build(),
    ]
}

/// One survey round's answers, with these Linear Issues.
fn queue_round_with(harness: &Harness, issues: Vec<LinearIssue>) {
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
            issues,
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
        // The fake fills in every member asked.
        asked: Vec::new(),
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
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            .any(|l| l == "evidence: merged acme/api#1"),
        "{:?}",
        detail.also
    );
    let landing = detail.landing.expect("a Landing Group section");
    assert!(
        landing
            .lines
            .iter()
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            .any(|l| l.text.contains("acme/api#1") && l.text.contains("merged")),
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

/// A cached issue excluded from Start still needs its missing member's state:
/// failed and closed reads must remain retryable until a merge is discovered.
#[test]
fn an_unoffered_cached_issue_reads_its_member_before_mark_for_release() {
    let mut config = support::harness::config_with_linear(&["acme"], &[("acme", "acme")]);
    config.linear[0].start_teams = vec!["TEAM".to_owned()];
    config.linear[0].release_state = Some(RELEASE.to_owned());
    let cached = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .pull_request(pr("acme/api", 1))
        .linear_viewer("acme", "acme")
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .unassigned()
                .listed_by(LinearSourceKind::StartTeamIssues)
                .state("Backlog", den::domain::facts::LinearStateKind::Backlog)
                .attached("acme/api", 1),
        )
        .build();
    let mut failed = Coverage::default();
    for kind in config.linear[0].linear_sources() {
        failed.record(
            Source::Owner {
                owner: "acme".to_owned(),
                kind: SourceKind::Linear(kind),
            },
            SourceState::Failed {
                error: "Linear unavailable".to_owned(),
            },
        );
    }
    let mut harness = Harness::with_cache(
        Ok(config),
        support::fakes::FakeCache::holding(den::cache::CachedObservation::fresh(cached)),
    );
    for _ in 0..4 {
        harness.answer(Ok(ObservationBuilder::new()
            .fully_read_owner("acme")
            .repo("acme/api")
            .build()));
        harness.linear.push_answer(LinearSurvey {
            coverage: failed.clone(),
            ..LinearSurvey::default()
        });
    }

    // The member disappeared from the complete open-PR survey; its read fails.
    harness.start();
    assert_eq!(
        harness.github.member_requests(),
        [MemberStatesRequest {
            pull_requests: vec![api_1()],
        }]
    );
    assert_eq!(detail_of(&mut harness, "TEAM-1").next_step, "none");
    for (state, step) in [
        (PullRequestState::Closed, "none"),
        (
            PullRequestState::Merged { merged_at: None },
            "Mark for release",
        ),
    ] {
        harness
            .github
            .push_member_states(member_states_answer(state));
        harness.settle(Event::Key(Key::Char('r')));
        assert_eq!(detail_of(&mut harness, "TEAM-1").next_step, step);
    }
    assert_eq!(harness.github.member_requests().len(), 3);

    // Merged evidence survives the next refresh and needs no further read.
    harness.settle(Event::Key(Key::Char('r')));
    assert_eq!(harness.github.member_requests().len(), 3);
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
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            .any(|l| l.text.contains("acme/app#2") && l.text.contains("lands with TEAM-2")),
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
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            .any(|u| u.contains("whether acme/api#1 merged or was closed without merging")),
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

/// Integration (#37 + #39): Land is a coordination step (#39's
/// `coordination_mode` gives it the approval-required mode), but a Land
/// Decision is a Linear Issue's, and `c` on a Linear Issue still refuses
/// before reading the workspace (until #40's Land Job), naming the Linear
/// Issue.
#[test]
fn c_on_a_land_linear_issue_refuses_without_touching_the_workspace() {
    let mut harness = harness();
    harness.start();
    detail_of(&mut harness, "TEAM-2");
    let effects = harness.key(Key::Char('c'));
    assert!(
        effects
            .iter()
            .all(|e| !matches!(e, Effect::InspectLaunch { .. } | Effect::Launch { .. })),
        "{effects:#?}"
    );
    let Some(Overlay::LaunchSheet(sheet)) = &harness.view.overlay else {
        panic!("a sheet: {:?}", harness.view.overlay);
    };
    assert_eq!(sheet.status, LaunchStatus::Refused);
    assert!(
        sheet
            .messages
            .iter()
            .any(|m| m.text.contains("Linear Issue")),
        "{:?}",
        sheet.messages
    );
}

fn member_states_coverage(harness: &Harness) -> Vec<SourceState> {
    let stored = harness.cache.stored();
    let last = stored.last().expect("cached");
    last.observation
        .coverage
        .entries()
        .iter()
        .filter(|e| e.source.is_member_states())
        .map(|e| e.state.clone())
        .collect()
}

/// A failed read leaves the Owner's member-state coverage failed. Once no
/// member needs reading (here api #1 is no longer attached), no read will
/// replace it, so den drops it: an old failure never leaves the picture
/// incomplete for good. And the state of a Pull Request that is no longer a
/// member is dropped too, so the kept states never outgrow the groups.
#[test]
fn member_state_facts_and_coverage_follow_the_groups_den_shows() {
    let mut harness = harness();
    // Round 1: the read fails (no queued answer).
    harness.start();
    assert!(
        member_states_coverage(&harness)
            .iter()
            .any(|s| !s.is_complete()),
        "the failure is recorded"
    );

    // Round 2: api #1 is read merged.
    queue_round(&harness);
    harness
        .github
        .push_member_states(member_states_answer(PullRequestState::Merged {
            merged_at: None,
        }));
    harness.settle(Event::Key(Key::Char('r')));
    let stored = harness.cache.stored();
    assert_eq!(stored.last().unwrap().observation.member_states.len(), 1);

    // Round 3: api #1 is detached from TEAM-1 in Linear.
    queue_round_with(
        &harness,
        vec![
            linear_issue("acme", "id-1", "TEAM-1").build(),
            linear_issue("acme", "id-2", "TEAM-2")
                .attached("acme/app", 2)
                .build(),
        ],
    );
    harness.settle(Event::Key(Key::Char('r')));
    assert_eq!(harness.github.member_requests().len(), 2, "nothing to read");
    let stored = harness.cache.stored();
    let last = stored.last().unwrap();
    assert!(
        last.observation.member_states.is_empty(),
        "{:?}",
        last.observation.member_states
    );
    assert!(member_states_coverage(&harness).is_empty());
}

/// An Owner's failed read is dropped once none of its members needs
/// reading, even when the members never got a state.
#[test]
fn a_failed_read_is_dropped_once_no_member_needs_reading() {
    let mut harness = harness();
    harness.start();
    assert_eq!(member_states_coverage(&harness).len(), 1);

    queue_round_with(
        &harness,
        vec![
            linear_issue("acme", "id-2", "TEAM-2")
                .attached("acme/app", 2)
                .build(),
        ],
    );
    harness.settle(Event::Key(Key::Char('r')));
    assert_eq!(harness.github.member_requests().len(), 1);
    assert!(member_states_coverage(&harness).is_empty());
}

/// Members den re-reads every round (read closed) come after members it
/// has never read, so the bounded read can't keep starving a new one.
#[test]
fn never_read_members_are_asked_for_before_closed_ones() {
    let mut harness = harness();
    harness
        .github
        .push_member_states(member_states_answer(PullRequestState::Closed));
    harness.start();

    queue_round_with(
        &harness,
        vec![
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/api", 5)
                .build(),
        ],
    );
    harness.settle(Event::Key(Key::Char('r')));
    let requests = harness.github.member_requests();
    assert_eq!(
        requests.last().unwrap().pull_requests,
        [PullRequestRef::new(RepoId::new("acme", "api"), 5), api_1()]
    );
}

/// Members den asked about and got no state for rotate behind the members
/// it never asked about, the longest unanswered first: the real read is
/// bounded (here as if it sent only one member a round), so one that never
/// resolves must not hold its place ahead of the rest every round.
#[test]
fn members_asked_without_an_answer_rotate_behind_the_others() {
    let api = |n: u64| PullRequestRef::new(RepoId::new("acme", "api"), n);
    let issue = || {
        vec![
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/api", 5)
                .attached("acme/api", 6)
                .build(),
        ]
    };
    let unanswered = |sent: PullRequestRef| MemberStatesRead {
        asked: vec![sent],
        ..MemberStatesRead::default()
    };
    let mut harness = harness_with_issues(issue());
    harness.github.push_member_states(unanswered(api(1)));
    harness.start();

    harness.set_clock(support::build::ts("2026-11-20T13:00:00Z"));
    queue_round_with(&harness, issue());
    harness.github.push_member_states(unanswered(api(5)));
    harness.settle(Event::Key(Key::Char('r')));

    harness.set_clock(support::build::ts("2026-11-20T14:00:00Z"));
    queue_round_with(&harness, issue());
    harness.github.push_member_states(unanswered(api(6)));
    harness.settle(Event::Key(Key::Char('r')));

    queue_round_with(&harness, issue());
    harness.settle(Event::Key(Key::Char('r')));

    let asked: Vec<Vec<PullRequestRef>> = harness
        .github
        .member_requests()
        .into_iter()
        .map(|r| r.pull_requests)
        .collect();
    assert_eq!(
        asked,
        [
            vec![api(1), api(5), api(6)],
            // #1 was asked and not answered: the never-asked go first.
            vec![api(5), api(6), api(1)],
            vec![api(6), api(1), api(5)],
            // All asked: the longest unanswered first.
            vec![api(1), api(5), api(6)],
        ]
    );
    let stored = harness.cache.stored();
    let attempts = &stored.last().unwrap().observation.member_attempts;
    assert_eq!(
        attempts.len(),
        3,
        "bounded by the members still wanted: {attempts:?}"
    );
}
