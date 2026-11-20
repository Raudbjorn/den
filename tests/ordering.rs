//! Ordering and Scope (pure): the list's order over every Work Item kind and
//! every Next Step.
//!
//! Most Next Steps cannot be produced through seam 1 yet (their kinds' rules are
//! sibling tickets), so this deck builds Decisions directly and goes through
//! `ordering::order`, the pure interface the App uses. Waiting On and the
//! updated-time order are also covered end to end through seam 2 in
//! `scope_and_sidebar.rs`.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, Explanation, Landing, WorkItem},
    domain::{NextStep, WaitingOn, WorkItemId, WorkflowId},
    ordering::{self, Scope},
};
use support::build::{repo_id, ts};

const UPDATED: &str = "2026-11-18T12:00:00Z";

/// A Work Item of `kind` in `repo`, told apart by `n`.
fn item(kind: &str, repo: &str, n: u64) -> WorkItemId {
    let repo = repo_id(repo);
    match kind {
        "issue" => WorkItemId::Issue { repo, number: n },
        "pr" => WorkItemId::PullRequest { repo, number: n },
        "branch" => WorkItemId::Branch {
            repo,
            name: format!("branch-{n}"),
        },
        "run" => WorkItemId::Run {
            repo,
            workflow: WorkflowId(format!("workflow-{n}")),
        },
        other => panic!("unknown kind {other}"),
    }
}

fn decision(
    id: WorkItemId,
    step: Option<NextStep>,
    waiting_on: WaitingOn,
    updated: &str,
) -> Decision {
    Decision {
        item: WorkItem {
            title: id.to_string(),
            url: String::new(),
            updated_at: ts(updated),
            id,
            identifier: None,
        },
        next_step: step,
        waiting_on,
        waiting_reason: String::new(),
        unattributed: false,
        explanation: Explanation::default(),
        coverage: Vec::new(),
        observed_at: ts(support::build::NOW),
        held_back: None,
        suggested_reviewers: None,
        landing: Landing::default(),
        start_order: None,
    }
}

fn ids(decisions: &[Decision]) -> Vec<String> {
    decisions.iter().map(|d| d.item.id.to_string()).collect()
}

#[test]
fn within_a_waiting_on_group_the_kind_order_holds_across_all_four_kinds() {
    // The spec's kind order (milestone 2's kinds included, spec 0002
    // "Ordering"); Fix CI is ranked by where it failed. A Land or Start
    // ranks by its Next Step alone, whatever the Work Item kind.
    let expected = vec![
        decision(
            item("run", "o/r", 1),
            Some(NextStep::FixCi),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "o/r", 2),
            Some(NextStep::ResolveConflicts),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "o/r", 3),
            Some(NextStep::AddressReview),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "o/r", 13),
            Some(NextStep::Land),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "o/r", 4),
            Some(NextStep::Merge),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "o/r", 5),
            Some(NextStep::Review),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "o/r", 14),
            Some(NextStep::RequestReview),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "o/r", 6),
            Some(NextStep::FixCi),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "o/r", 7),
            Some(NextStep::FinishDraft),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("issue", "o/r", 15),
            Some(NextStep::MarkForRelease),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("issue", "o/r", 8),
            Some(NextStep::Implement),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("issue", "o/r", 16),
            Some(NextStep::Start),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("issue", "o/r", 9),
            Some(NextStep::Triage),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("branch", "o/r", 10),
            Some(NextStep::OpenPr),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("branch", "o/r", 11),
            Some(NextStep::PruneOrRevive),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(item("issue", "o/r", 12), None, WaitingOn::Me, UPDATED),
    ];
    // Every Next Step appears, so a new one cannot slip in unranked.
    for step in NextStep::ALL {
        assert!(
            expected.iter().any(|d| d.next_step == Some(step)),
            "{step:?}"
        );
    }
    let mut shuffled = expected.clone();
    shuffled.reverse();
    shuffled.rotate_left(5);
    assert_eq!(ids(&ordering::order(shuffled)), ids(&expected));
}

#[test]
fn waiting_on_comes_before_kind_and_kind_before_updated_time() {
    let you_old_triage = decision(
        item("issue", "o/r", 1),
        Some(NextStep::Triage),
        WaitingOn::Me,
        "2026-01-01T00:00:00Z",
    );
    let nobody_fix_ci_on_main = decision(
        item("run", "o/r", 2),
        Some(NextStep::FixCi),
        WaitingOn::Nobody,
        "2026-11-19T00:00:00Z",
    );
    let someone_conflicts = decision(
        item("pr", "o/r", 3),
        Some(NextStep::ResolveConflicts),
        WaitingOn::SomeoneElse,
        "2026-11-19T00:00:00Z",
    );
    let you_new_merge = decision(
        item("pr", "o/r", 4),
        Some(NextStep::Merge),
        WaitingOn::Me,
        "2026-11-01T00:00:00Z",
    );
    let you_newer_merge = decision(
        item("pr", "p/q", 5),
        Some(NextStep::Merge),
        WaitingOn::Me,
        "2026-11-10T00:00:00Z",
    );
    let ordered = ordering::order(vec![
        someone_conflicts.clone(),
        nobody_fix_ci_on_main.clone(),
        you_old_triage.clone(),
        you_new_merge.clone(),
        you_newer_merge.clone(),
    ]);
    assert_eq!(
        ids(&ordered),
        ids(&[
            you_newer_merge,
            you_new_merge,
            you_old_triage,
            nobody_fix_ci_on_main,
            someone_conflicts,
        ])
    );
}

#[test]
fn scope_keeps_order_and_matches_owners_and_repos_case_insensitively() {
    let decisions = ordering::order(vec![
        decision(
            item("pr", "Octo/App", 1),
            Some(NextStep::Merge),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("issue", "octo/lib", 2),
            Some(NextStep::Triage),
            WaitingOn::Me,
            UPDATED,
        ),
        decision(
            item("pr", "other/app", 3),
            Some(NextStep::Review),
            WaitingOn::Me,
            UPDATED,
        ),
    ]);
    let in_scope = |scope: Scope| -> Vec<String> {
        ordering::in_scope(&decisions, &scope)
            .map(|d| d.item.id.to_string())
            .collect()
    };
    assert_eq!(in_scope(Scope::AllOwners).len(), 3);
    assert_eq!(
        in_scope(Scope::Owner("OCTO".to_owned())),
        ["Octo/App#1", "octo/lib#2"]
    );
    assert_eq!(in_scope(Scope::Repo(repo_id("octo/app"))), ["Octo/App#1"]);
    assert!(in_scope(Scope::Repo(repo_id("octo/none"))).is_empty());
}
