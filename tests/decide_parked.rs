//! Seam 1, Parked Linear Issues (#74, spec 0004 stories 64–65): a Linear
//! Workflow's `parked_states` hold a Linear Issue in one of them back from
//! Implement and Start. It has no Next Step, is Waiting On nobody, and says
//! "parked in <state>". The rule runs after the Landing Group rules, so a
//! parked issue whose group could land still shows Land.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        LinearWorkflowPolicy, NextStep, Observation, Policies, ReviewPolicy, StartPolicy, Viewers,
        WaitingOn,
        facts::{LinearStateKind, ReviewVerdict},
    },
};
use support::build::{ME, ObservationBuilder, PrBuilder, linear_issue, now, pr};

const PAUSED: &str = "Paused";

/// acme's Linear Workflow: start team TEAM, review rules, and `Paused` parked.
fn parked() -> Policies {
    workflow(&[PAUSED])
}

/// The same workflow naming no parked state.
fn not_parked() -> Policies {
    workflow(&[])
}

fn workflow(parked_states: &[&str]) -> Policies {
    Policies::default().with(
        "acme",
        LinearWorkflowPolicy {
            start: StartPolicy {
                teams: vec!["TEAM".to_owned()],
                include_backlog: false,
                active_cycle_only: false,
            },
            review: Some(ReviewPolicy {
                exact_head_approval: true,
                comment_reviews_need_rereview: true,
                pending_requests_hold_merge: true,
                reviewers: Vec::new(),
            }),
            parked_states: parked_states.iter().map(|s| (*s).to_owned()).collect(),
            ..LinearWorkflowPolicy::default()
        },
    )
}

fn acme_owner() -> ObservationBuilder {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .linear_viewer("acme", "acme")
}

/// A PR of mine that would Merge: passing, mergeable, approved at its head.
fn ready(repo: &str, number: u64) -> PrBuilder {
    pr(repo, number)
        .author(ME)
        .passing("ci")
        .approved()
        .review_at_head("alice", ReviewVerdict::Approved)
}

fn decide_all(observation: &Observation, policies: &Policies) -> Vec<Decision> {
    decide(observation, &Viewers::of(observation), policies, now())
}

fn linear<'d>(decisions: &'d [Decision], identifier: &str) -> &'d Decision {
    decisions
        .iter()
        .find(|d| d.item.identifier.as_deref() == Some(identifier))
        .unwrap_or_else(|| panic!("no Decision for {identifier}: {decisions:#?}"))
}

fn assert_parked(decision: &Decision, state: &str) {
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.waiting_on, WaitingOn::Nobody, "{decision:#?}");
    assert!(!decision.unattributed, "{decision:#?}");
    assert_eq!(
        decision.explanation.no_step_reason.as_deref(),
        Some(format!("parked in {state}").as_str()),
        "{decision:#?}"
    );
    assert!(decision.explanation.winning_rule.is_none(), "{decision:#?}");
}

/// Acceptance: an assigned Linear Issue without a PR in a parked state has no
/// Next Step, Waiting On nobody, "parked in <state>". State names compare as
/// the release state does, ignoring case; the reason gives Linear's spelling.
#[test]
fn an_assigned_issue_in_a_parked_state_has_no_next_step_and_waits_on_nobody() {
    let observation = acme_owner()
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1").state(PAUSED, LinearStateKind::Started),
        )
        .linear_issue(
            linear_issue("acme", "id-2", "TEAM-2").state("paused", LinearStateKind::Started),
        )
        .build();
    let decisions = decide_all(&observation, &parked());

    assert_parked(linear(&decisions, "TEAM-1"), PAUSED);
    assert_parked(linear(&decisions, "TEAM-2"), "paused");
}

/// Acceptance: the same issue outside a parked state is Implement, as before;
/// and a workflow naming no parked state never parks anything.
#[test]
fn outside_a_parked_state_or_without_parked_states_the_issue_is_implement() {
    let in_progress = acme_owner()
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .build();
    let implement = linear(&decide_all(&in_progress, &parked()), "TEAM-1").clone();
    assert_eq!(
        implement.next_step,
        Some(NextStep::Implement),
        "{implement:#?}"
    );
    assert_eq!(implement.waiting_on, WaitingOn::Me);

    let paused = acme_owner()
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1").state(PAUSED, LinearStateKind::Started),
        )
        .build();
    let unparked = linear(&decide_all(&paused, &not_parked()), "TEAM-1").clone();
    assert_eq!(
        unparked.next_step,
        Some(NextStep::Implement),
        "{unparked:#?}"
    );
}

/// The rule comes before Start too: an unassigned issue the start team offers
/// is parked when its state is.
#[test]
fn an_offered_unassigned_issue_in_a_parked_state_is_not_start() {
    let observation = acme_owner()
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .unassigned()
                .state(PAUSED, LinearStateKind::Unstarted),
        )
        .linear_issue(
            linear_issue("acme", "id-2", "TEAM-2")
                .unassigned()
                .state("Todo", LinearStateKind::Unstarted),
        )
        .build();
    let decisions = decide_all(&observation, &parked());

    assert_parked(linear(&decisions, "TEAM-1"), PAUSED);
    assert_eq!(
        linear(&decisions, "TEAM-2").next_step,
        Some(NextStep::Start),
        "the unparked one is still offered"
    );
}

/// Acceptance: a parked issue whose Landing Group could land is Land. The
/// group rules come first.
#[test]
fn a_parked_issue_whose_group_could_land_is_land() {
    let observation = acme_owner()
        .pull_request(ready("acme/api", 1))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .state(PAUSED, LinearStateKind::Started)
                .attached("acme/api", 1),
        )
        .build();
    let decisions = decide_all(&observation, &parked());

    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, Some(NextStep::Land), "{issue:#?}");
    assert_eq!(issue.waiting_on, WaitingOn::Me);
}
