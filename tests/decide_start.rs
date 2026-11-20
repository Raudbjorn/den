//! Seam 1, Start (#33): an unassigned Linear Issue in one of its Owner's start
//! teams, in an unstarted state (or backlog when the workflow includes it),
//! and in the active cycle when the workflow asks, gives **Start**, Waiting
//! On nobody. Any other unassigned Linear Issue gets no Decision. Starts
//! order by priority, then active cycle, then oldest first.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        LinearWorkflowPolicy, NextStep, Observation, Policies, StartPolicy, Viewers, WaitingOn,
        facts::{LinearPriority, LinearStateKind},
    },
    ordering,
};
use support::build::{ME, ObservationBuilder, issue, linear_issue, now, pr};

/// `acme`'s Linear Workflow offers team `TEAM`'s unstarted issues.
fn start_policy(include_backlog: bool, active_cycle_only: bool) -> Policies {
    Policies::default().with(
        "acme",
        LinearWorkflowPolicy {
            start: StartPolicy {
                teams: vec!["TEAM".to_owned()],
                include_backlog,
                active_cycle_only,
            },
        },
    )
}

fn decide_under(observation: &Observation, policies: &Policies) -> Vec<Decision> {
    decide(observation, &Viewers::of(observation), policies, now())
}

fn find<'d>(decisions: &'d [Decision], identifier: &str) -> Option<&'d Decision> {
    decisions
        .iter()
        .find(|d| d.item.identifier.as_deref() == Some(identifier))
}

fn identifiers(decisions: &[Decision]) -> Vec<&str> {
    decisions
        .iter()
        .filter_map(|d| d.item.identifier.as_deref())
        .collect()
}

fn observation(issues: Vec<support::build::LinearIssueBuilder>) -> Observation {
    issues
        .into_iter()
        .fold(
            ObservationBuilder::new()
                .fully_read_owner("acme")
                .linear_viewer("acme", "acme"),
            ObservationBuilder::linear_issue,
        )
        .build()
}

#[test]
fn an_unassigned_unstarted_issue_in_a_start_team_is_start_waiting_on_nobody() {
    let observation = observation(vec![linear_issue("acme", "id-1", "TEAM-1").up_for_grabs()]);
    let decisions = decide_under(&observation, &start_policy(false, false));

    let start = find(&decisions, "TEAM-1").expect("a Decision");
    assert_eq!(start.next_step, Some(NextStep::Start));
    assert_eq!(start.waiting_on, WaitingOn::Nobody);
    assert!(!start.unattributed, "nobody is the rule, not a gap");
    let rule = start.explanation.winning_rule.as_deref().unwrap();
    assert!(
        rule.contains("unassigned") && rule.contains("start team TEAM") && rule.contains("Todo"),
        "{rule}"
    );
    assert!(
        start
            .explanation
            .unknowns
            .iter()
            .any(|u| u.contains("attachments")),
        "until #35, a PR may exist: {start:?}"
    );
    assert!(start.start_order.is_some());
}

#[test]
fn a_start_team_matches_its_key_whatever_the_case() {
    let observation = observation(vec![linear_issue("acme", "id-1", "TEAM-1").up_for_grabs()]);
    let policies = Policies::default().with(
        "ACME",
        LinearWorkflowPolicy {
            start: StartPolicy {
                teams: vec!["team".to_owned()],
                ..StartPolicy::default()
            },
        },
    );
    let decisions = decide_under(&observation, &policies);
    assert_eq!(
        find(&decisions, "TEAM-1").and_then(|d| d.next_step),
        Some(NextStep::Start)
    );
}

#[test]
fn an_issue_in_another_team_does_not_appear() {
    let observation = observation(vec![
        linear_issue("acme", "id-1", "TEAM-1").up_for_grabs(),
        linear_issue("acme", "id-2", "OPS-2").up_for_grabs(),
    ]);
    let decisions = decide_under(&observation, &start_policy(false, false));
    assert_eq!(identifiers(&decisions), ["TEAM-1"]);
}

#[test]
fn only_unstarted_states_are_offered_and_backlog_only_when_enabled() {
    let issues = || {
        vec![
            linear_issue("acme", "id-1", "TEAM-1").up_for_grabs(),
            linear_issue("acme", "id-2", "TEAM-2")
                .unassigned()
                .state("Backlog", LinearStateKind::Backlog),
            linear_issue("acme", "id-3", "TEAM-3")
                .unassigned()
                .state("Triage", LinearStateKind::Triage),
            linear_issue("acme", "id-4", "TEAM-4")
                .unassigned()
                .state("In Progress", LinearStateKind::Started),
            linear_issue("acme", "id-5", "TEAM-5")
                .unassigned()
                .state("Done", LinearStateKind::Completed),
            linear_issue("acme", "id-6", "TEAM-6")
                .unassigned()
                .state("Odd", LinearStateKind::Other("odd".to_owned())),
        ]
    };

    let without_backlog = decide_under(&observation(issues()), &start_policy(false, false));
    assert_eq!(identifiers(&without_backlog), ["TEAM-1"]);

    let with_backlog = decide_under(&observation(issues()), &start_policy(true, false));
    let mut offered = identifiers(&with_backlog);
    offered.sort_unstable();
    assert_eq!(offered, ["TEAM-1", "TEAM-2"]);
    let backlog = find(&with_backlog, "TEAM-2").unwrap();
    assert_eq!(backlog.next_step, Some(NextStep::Start));
    assert!(
        backlog
            .explanation
            .winning_rule
            .as_deref()
            .is_some_and(|r| r.contains("backlog")),
        "{backlog:?}"
    );
}

#[test]
fn the_active_cycle_limit_is_honoured() {
    let issues = || {
        vec![
            linear_issue("acme", "id-1", "TEAM-1")
                .up_for_grabs()
                .cycle(3, true),
            linear_issue("acme", "id-2", "TEAM-2")
                .up_for_grabs()
                .cycle(4, false),
            linear_issue("acme", "id-3", "TEAM-3").up_for_grabs(),
        ]
    };

    let limited = decide_under(&observation(issues()), &start_policy(false, true));
    assert_eq!(identifiers(&limited), ["TEAM-1"]);
    let start = find(&limited, "TEAM-1").unwrap();
    assert!(
        start
            .explanation
            .other_conditions
            .iter()
            .any(|c| c.contains("active cycle") && c.contains("cycle 3")),
        "{start:?}"
    );

    let unlimited = decide_under(&observation(issues()), &start_policy(false, false));
    assert_eq!(unlimited.len(), 3, "{unlimited:#?}");
    assert!(
        unlimited
            .iter()
            .all(|d| d.next_step == Some(NextStep::Start))
    );
}

/// The empty policy (an Owner whose workflow names no start team, or none at
/// all) offers nothing: an unassigned Linear Issue gets no Decision.
#[test]
fn without_start_teams_nothing_is_offered() {
    let observation = observation(vec![linear_issue("acme", "id-1", "TEAM-1").up_for_grabs()]);
    let decisions = decide_under(&observation, &Policies::default());
    assert!(identifiers(&decisions).is_empty(), "{decisions:#?}");
}

/// An assigned issue in a start team stays Implement: Start is for unassigned
/// work only.
#[test]
fn an_assigned_issue_in_a_start_team_is_still_implement() {
    let observation = observation(vec![
        linear_issue("acme", "id-1", "TEAM-1").state("Todo", LinearStateKind::Unstarted),
    ]);
    let decisions = decide_under(&observation, &start_policy(true, false));
    let decision = find(&decisions, "TEAM-1").unwrap();
    assert_eq!(decision.next_step, Some(NextStep::Implement));
    assert_eq!(decision.waiting_on, WaitingOn::Me);
    assert_eq!(decision.start_order, None);
}

/// Spec 0002, "Ordering": within Start, priority (urgent first, no priority
/// last), then the active cycle first, then oldest first; whatever was
/// updated last.
#[test]
fn starts_order_by_priority_then_active_cycle_then_age() {
    let issue = |id: &str, identifier: &str| {
        linear_issue("acme", id, identifier)
            .up_for_grabs()
            // Update times run against the expected order, so they can't
            // be what orders them.
            .updated(&format!("2026-11-1{}T12:00:00Z", &id[3..]))
    };
    let observation = observation(vec![
        issue("id-1", "TEAM-1").priority(LinearPriority::None),
        issue("id-2", "TEAM-2").priority(LinearPriority::Low),
        issue("id-3", "TEAM-3")
            .priority(LinearPriority::High)
            .created("2026-11-05T12:00:00Z"),
        issue("id-4", "TEAM-4")
            .priority(LinearPriority::High)
            .created("2026-11-02T12:00:00Z"),
        issue("id-5", "TEAM-5")
            .priority(LinearPriority::High)
            .cycle(7, true)
            .created("2026-11-09T12:00:00Z"),
        issue("id-6", "TEAM-6").priority(LinearPriority::Medium),
        issue("id-7", "TEAM-7").priority(LinearPriority::Urgent),
    ]);
    let ordered = ordering::order(decide_under(&observation, &start_policy(false, false)));
    assert_eq!(
        identifiers(&ordered),
        [
            "TEAM-7", // urgent
            "TEAM-5", // high, active cycle
            "TEAM-4", // high, older
            "TEAM-3", // high, newer
            "TEAM-6", // medium
            "TEAM-2", // low
            "TEAM-1", // no priority
        ]
    );
}

/// Start waits on nobody, so it orders with the other nobody items by kind:
/// after Implement and before Triage (spec 0002's kind order), whatever was
/// updated last.
#[test]
fn start_takes_its_place_in_the_kind_order() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .linear_viewer("acme", "acme")
        .issue(
            issue("acme/api", 1)
                .label("bug")
                .updated("2026-11-10T12:00:00Z"),
        )
        .issue(issue("acme/api", 2).updated("2026-11-19T12:00:00Z"))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .up_for_grabs()
                .updated("2026-11-18T12:00:00Z"),
        )
        .build();
    let ordered = ordering::order(decide_under(&observation, &start_policy(false, false)));
    let steps: Vec<(WaitingOn, Option<NextStep>)> = ordered
        .iter()
        .map(|d| (d.waiting_on, d.next_step))
        .collect();
    assert_eq!(
        steps,
        [
            (WaitingOn::Nobody, Some(NextStep::Implement)),
            (WaitingOn::Nobody, Some(NextStep::Start)),
            (WaitingOn::Nobody, Some(NextStep::Triage)),
        ]
    );
}

/// Start never touches a GitHub Work Item's Decision.
#[test]
fn start_leaves_github_decisions_as_they_were() {
    let github_only = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .pull_request(pr("acme/api", 4).author(ME).draft())
        .build();
    let mut with_start = github_only.clone();
    with_start.linear.issues.push(
        linear_issue("acme", "id-1", "TEAM-1")
            .up_for_grabs()
            .build(),
    );

    let before = decide_under(&github_only, &Policies::default());
    let after: Vec<Decision> = decide_under(&with_start, &start_policy(true, true))
        .into_iter()
        .filter(|d| d.item.identifier.is_none())
        .collect();
    assert_eq!(before, after);
}
