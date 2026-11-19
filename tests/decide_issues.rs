//! Seam 1: the decision engine on Issues. Observation in, Decisions out.
//!
//! Rules (spec, "Next Step rules"): a linked open PR → no Next Step; no labels
//! and no assignee → Triage; assigned, or labelled but unassigned → Implement.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        Incompleteness, NextStep, Observation, SourceKind, SourceState, WaitingOn, WorkItemId,
        facts::{LinkKind, LinkedPrState},
    },
};
use support::build::{IssueBuilder, ME, ObservationBuilder, issue, now, pr, repo_id};

use LinkKind::{ClosingReference, ManualLink};
use LinkedPrState::{Closed, Merged, Open};

fn decisions(observation: &Observation) -> Vec<Decision> {
    decide(observation, &observation.viewer, now())
}

fn issue_id(repo: &str, number: u64) -> WorkItemId {
    WorkItemId::Issue {
        repo: repo_id(repo),
        number,
    }
}

fn find<'a>(decisions: &'a [Decision], id: &WorkItemId) -> &'a Decision {
    decisions
        .iter()
        .find(|d| &d.item.id == id)
        .unwrap_or_else(|| panic!("no decision for {id}"))
}

/// Decides one Issue `o/r#1` in an Observation where everything was read.
fn decide_one(case: IssueBuilder) -> Decision {
    let observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .issue(case)
        .build();
    find(&decisions(&observation), &issue_id("o/r", 1)).clone()
}

fn has(lines: &[String], needle: &str) -> bool {
    lines.iter().any(|l| l.contains(needle))
}

struct Expect {
    step: Option<NextStep>,
    waiting_on: WaitingOn,
    /// Substring of the winning rule (with a step) or of the no-step reason.
    why: &'static str,
    /// Substring of the Waiting On reason.
    waiting_reason: &'static str,
}

#[test]
fn issue_rule_deck() {
    let cases = [
        (
            "no labels, no assignee",
            issue("o/r", 1),
            Expect {
                step: Some(NextStep::Triage),
                waiting_on: WaitingOn::Nobody,
                why: "no labels and no assignee",
                waiting_reason: "nobody",
            },
        ),
        (
            "assigned to me",
            issue("o/r", 1).assignee(ME),
            Expect {
                step: Some(NextStep::Implement),
                waiting_on: WaitingOn::Me,
                why: "assigned to me",
                waiting_reason: "you are the assignee",
            },
        ),
        (
            "assigned to me, login in another case",
            issue("o/r", 1).assignee("ME"),
            Expect {
                step: Some(NextStep::Implement),
                waiting_on: WaitingOn::Me,
                why: "assigned to ME",
                waiting_reason: "you are the assignee",
            },
        ),
        (
            "assigned to someone else",
            issue("o/r", 1).assignee("alice").label("bug"),
            Expect {
                step: Some(NextStep::Implement),
                waiting_on: WaitingOn::SomeoneElse,
                why: "assigned to alice",
                waiting_reason: "alice",
            },
        ),
        (
            "labelled and unassigned",
            issue("o/r", 1).label("bug").label("ui"),
            Expect {
                step: Some(NextStep::Implement),
                waiting_on: WaitingOn::Nobody,
                why: "labelled (bug, ui) but unassigned",
                waiting_reason: "nobody is assigned",
            },
        ),
        (
            "me as one of several assignees",
            issue("o/r", 1)
                .assignee("alice")
                .assignee(ME)
                .assignee("bob"),
            Expect {
                step: Some(NextStep::Implement),
                waiting_on: WaitingOn::Me,
                why: "assigned to alice, me, bob",
                waiting_reason: "you are one of the assignees",
            },
        ),
        (
            "several assignees, none of them me",
            issue("o/r", 1).assignee("alice").assignee("bob"),
            Expect {
                step: Some(NextStep::Implement),
                waiting_on: WaitingOn::SomeoneElse,
                why: "assigned to alice, bob",
                waiting_reason: "not one of the assignees",
            },
        ),
        (
            "suppressed by a manual link",
            issue("o/r", 1).linked(ManualLink, "o/r", 9, Open),
            Expect {
                step: None,
                waiting_on: WaitingOn::Nobody,
                why: "an open linked PR carries the work: o/r#9",
                waiting_reason: "open linked PR",
            },
        ),
        (
            "suppressed by a closing reference",
            issue("o/r", 1)
                .assignee(ME)
                .linked(ClosingReference, "o/r", 9, Open),
            Expect {
                step: None,
                waiting_on: WaitingOn::Nobody,
                why: "an open linked PR carries the work: o/r#9",
                waiting_reason: "open linked PR",
            },
        ),
        (
            "a closed linked PR does not suppress",
            issue("o/r", 1).linked(ClosingReference, "o/r", 9, Closed),
            Expect {
                step: Some(NextStep::Triage),
                waiting_on: WaitingOn::Nobody,
                why: "no labels and no assignee",
                waiting_reason: "nobody",
            },
        ),
        (
            "a merged linked PR does not suppress",
            issue("o/r", 1)
                .assignee(ME)
                .linked(ManualLink, "o/r", 9, Merged),
            Expect {
                step: Some(NextStep::Implement),
                waiting_on: WaitingOn::Me,
                why: "assigned to me",
                waiting_reason: "you are the assignee",
            },
        ),
    ];
    for (name, case, expect) in cases {
        let decision = decide_one(case);
        assert_eq!(decision.next_step, expect.step, "{name}: step");
        assert_eq!(decision.waiting_on, expect.waiting_on, "{name}: waiting on");
        assert!(!decision.unattributed, "{name}: attributed");
        assert!(
            decision.waiting_reason.contains(expect.waiting_reason),
            "{name}: waiting reason {:?}",
            decision.waiting_reason
        );
        let why = match expect.step {
            Some(_) => decision.explanation.winning_rule.clone(),
            None => decision.explanation.no_step_reason.clone(),
        };
        assert!(
            why.as_deref().is_some_and(|w| w.contains(expect.why)),
            "{name}: explanation {:?}",
            decision.explanation
        );
        assert!(
            decision.explanation.unknowns.is_empty(),
            "{name}: nothing unknown {:?}",
            decision.explanation.unknowns
        );
    }
}

#[test]
fn a_suppressed_issue_says_what_it_would_otherwise_be() {
    let decision = decide_one(issue("o/r", 1).assignee(ME).linked(
        ClosingReference,
        "o/r",
        9,
        Open,
    ));
    assert!(
        has(
            &decision.explanation.other_conditions,
            "would be Implement: the Issue is assigned to me"
        ),
        "{:?}",
        decision.explanation.other_conditions
    );
}

#[test]
fn a_closed_linked_pr_is_still_listed_and_said_not_to_carry_the_work() {
    let decision = decide_one(issue("o/r", 1).linked(ClosingReference, "o/r", 9, Closed));
    let also = &decision.explanation.other_conditions;
    assert!(
        has(also, "linked PR o/r#9 (closing reference, closed)"),
        "{also:?}"
    );
    assert!(has(also, "no linked PR is open"), "{also:?}");
}

#[test]
fn a_mere_mention_does_not_suppress() {
    // An open PR in the same Repo mentions the Issue in its title and branch
    // name, but GitHub records no link: mentions never become linked PRs.
    let mut observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .issue(issue("o/r", 1).title("Crash on start"))
        .build();
    let mut mentioning = pr("o/r", 9).title("Fix #1: crash on start").build();
    mentioning.head_ref = "1-crash-on-start".to_owned();
    observation.pull_requests.push(mentioning);
    let decision = find(&decisions(&observation), &issue_id("o/r", 1)).clone();
    assert_eq!(decision.next_step, Some(NextStep::Triage));
    assert!(
        !has(&decision.explanation.other_conditions, "o/r#9"),
        "{:?}",
        decision.explanation
    );
}

#[test]
fn every_linked_pr_is_listed_with_its_repo_and_state_not_just_the_first() {
    let decision = decide_one(
        issue("o/r", 1)
            .linked(ClosingReference, "o/r", 9, Open)
            .linked(ClosingReference, "o/r", 4, Closed)
            .linked(ManualLink, "o/lib", 12, Open)
            .linked(ManualLink, "o/lib", 3, Merged),
    );
    assert_eq!(decision.next_step, None);
    let reason = decision.explanation.no_step_reason.clone().unwrap();
    assert!(
        reason.contains("o/r#9") && reason.contains("o/lib#12"),
        "{reason}"
    );
    assert!(
        !reason.contains("o/r#4") && !reason.contains("o/lib#3"),
        "{reason}"
    );
    let also = &decision.explanation.other_conditions;
    for line in [
        "linked PR o/r#9 (closing reference, open)",
        "linked PR o/r#4 (closing reference, closed)",
        "linked PR o/lib#12 (manual link, open)",
        "linked PR o/lib#3 (manual link, merged)",
    ] {
        assert!(has(also, line), "missing {line:?} in {also:?}");
    }
}

#[test]
fn a_linked_pr_in_another_repo_outside_the_watched_owners_is_named() {
    // "elsewhere" is not a watched Owner, so its PR is outside every Scope.
    let observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .issue(issue("o/r", 1).linked(ManualLink, "elsewhere/lib", 77, Open))
        .build();
    assert!(!observation.owners.contains(&"elsewhere".to_owned()));
    let decision = find(&decisions(&observation), &issue_id("o/r", 1)).clone();
    assert_eq!(decision.next_step, None);
    assert!(
        decision
            .explanation
            .no_step_reason
            .as_deref()
            .is_some_and(|r| r.contains("elsewhere/lib#77")),
        "{:?}",
        decision.explanation
    );
    assert!(has(
        &decision.explanation.other_conditions,
        "linked PR elsewhere/lib#77 (manual link, open)"
    ));
}

#[test]
fn a_pr_linked_both_ways_is_listed_once_with_both_relationships() {
    let decision = decide_one(
        issue("o/r", 1)
            .linked(ClosingReference, "o/r", 9, Open)
            .linked(ManualLink, "o/r", 9, Open),
    );
    let also = &decision.explanation.other_conditions;
    let mentions = also.iter().filter(|l| l.contains("o/r#9")).count();
    assert_eq!(mentions, 1, "{also:?}");
    assert!(
        has(
            also,
            "linked PR o/r#9 (closing reference and manual link, open)"
        ),
        "{also:?}"
    );
}

#[test]
fn unread_linked_prs_are_an_unknown() {
    let decision = decide_one(
        issue("o/r", 1)
            .linked(ClosingReference, "o/r", 4, Closed)
            .more_links_unread(),
    );
    assert_eq!(decision.next_step, Some(NextStep::Triage));
    assert!(
        has(
            &decision.explanation.unknowns,
            "more linked PRs than den read"
        ),
        "{:?}",
        decision.explanation.unknowns
    );
}

#[test]
fn an_assigned_and_labelled_issue_mentions_its_labels() {
    let decision = decide_one(issue("o/r", 1).assignee(ME).label("bug"));
    assert_eq!(decision.next_step, Some(NextStep::Implement));
    assert!(has(&decision.explanation.other_conditions, "labelled bug"));
}

#[test]
fn issues_are_qualified_by_owner_and_repo() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .issue(issue("alpha/app", 1))
        .issue(issue("beta/app", 1).assignee(ME))
        .build();
    let decisions = decisions(&observation);
    assert_eq!(decisions.len(), 2);
    assert_eq!(
        find(&decisions, &issue_id("alpha/app", 1)).next_step,
        Some(NextStep::Triage)
    );
    assert_eq!(
        find(&decisions, &issue_id("beta/app", 1)).next_step,
        Some(NextStep::Implement)
    );
}

#[test]
fn partial_issue_coverage_is_carried_onto_issues_only() {
    let partial = SourceState::Partial(Incompleteness::ErrorsAlongsideData {
        errors: vec!["Resource not accessible by integration".to_owned()],
    });
    let observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .repo_coverage("o/r", SourceKind::Issues, partial.clone())
        .issue(issue("o/r", 1))
        .pull_request(pr("o/r", 2))
        .build();
    let decisions = decisions(&observation);
    let incomplete = |id: &WorkItemId| -> Vec<SourceState> {
        find(&decisions, id)
            .coverage
            .iter()
            .filter(|c| !c.state.is_complete())
            .map(|c| c.state.clone())
            .collect()
    };
    assert_eq!(incomplete(&issue_id("o/r", 1)), vec![partial]);
    assert_eq!(
        incomplete(&WorkItemId::PullRequest {
            repo: repo_id("o/r"),
            number: 2
        }),
        Vec::<SourceState>::new()
    );
}
