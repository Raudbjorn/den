//! Seam 1, Linear Issues (#32): an assigned Linear Issue gives Implement,
//! Waiting On the Linear assignee, "me" decided by the Linear viewer's user
//! ID (never a GitHub login); a Linear Issue is identified by workspace and
//! permanent ID; and Linear Issues never change a GitHub Work Item's Decision.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        LinearIssueId, LinearSourceKind, NextStep, Observation, Policies, Source, SourceKind,
        SourceState, Viewers, WaitingOn, WorkItemId, WorkItemKind,
    },
    ordering::{Scope, in_scope, same_work_item},
};
use support::build::{LINEAR_ME_NAME, ME, ObservationBuilder, issue, linear_issue, now, pr};

fn decide_all(observation: &Observation) -> Vec<Decision> {
    decide(
        observation,
        &Viewers::of(observation),
        &Policies::default(),
        now(),
    )
}

fn decision_for<'d>(decisions: &'d [Decision], identifier: &str) -> &'d Decision {
    decisions
        .iter()
        .find(|d| d.item.identifier.as_deref() == Some(identifier))
        .unwrap_or_else(|| panic!("no Decision for {identifier}: {decisions:#?}"))
}

#[test]
fn an_assigned_linear_issue_is_implement_waiting_on_its_linear_assignee() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2").assignee("user-ada", "Ada"))
        .build();
    let decisions = decide_all(&observation);

    let mine = decision_for(&decisions, "TEAM-1");
    assert_eq!(mine.next_step, Some(NextStep::Implement));
    assert_eq!(mine.waiting_on, WaitingOn::Me);
    assert!(!mine.unattributed);
    assert!(mine.waiting_reason.contains(LINEAR_ME_NAME), "{mine:?}");
    assert!(
        mine.explanation
            .winning_rule
            .as_deref()
            .is_some_and(|r| r.contains("assigned")),
        "{mine:?}"
    );

    let theirs = decision_for(&decisions, "TEAM-2");
    assert_eq!(theirs.next_step, Some(NextStep::Implement));
    assert_eq!(theirs.waiting_on, WaitingOn::SomeoneElse);
    assert!(theirs.waiting_reason.contains("Ada"), "{theirs:?}");
}

/// Story 37: "me" for a Linear Issue is the Linear account, never a name that
/// happens to match the GitHub login.
#[test]
fn a_linear_user_named_like_my_github_login_is_not_me() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").assignee("user-other", ME))
        .build();
    let decisions = decide_all(&observation);
    let decision = decision_for(&decisions, "TEAM-1");
    assert_eq!(decision.waiting_on, WaitingOn::SomeoneElse, "{decision:?}");
}

#[test]
fn without_a_linear_viewer_den_cannot_say_whose_it_is() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .owner_coverage(
            "acme",
            SourceKind::Linear(LinearSourceKind::Viewer),
            SourceState::Failed {
                error: "Linear refused the API key".to_owned(),
            },
        )
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .build();
    let decisions = decide_all(&observation);
    let decision = decision_for(&decisions, "TEAM-1");
    assert_eq!(decision.next_step, Some(NextStep::Implement));
    assert_eq!(decision.waiting_on, WaitingOn::Nobody);
    assert!(decision.unattributed, "{decision:?}");
    assert!(
        decision.waiting_reason.contains("who you are"),
        "{decision:?}"
    );
    assert!(
        decision.coverage.iter().any(|c| !c.state.is_complete()),
        "the failed viewer is this item's coverage: {decision:?}"
    );
}

/// Changed by #33. Under #32 an unassigned Linear Issue had a Decision with
/// no Next Step ("Start is #33"). Now Start is decided from the Owner's start
/// teams (`decide_start.rs`), and an unassigned issue the workflow doesn't
/// offer is not den's to show: the Linear source never asks for one, and
/// one kept from an earlier survey gets no Decision. Under the empty policy
/// (no start teams) that is every unassigned issue.
#[test]
fn an_unassigned_linear_issue_no_start_team_offers_gets_no_decision() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").unassigned())
        .build();
    let decisions = decide_all(&observation);
    assert!(
        decisions
            .iter()
            .all(|d| d.item.identifier.as_deref() != Some("TEAM-1")),
        "{decisions:#?}"
    );
}

/// #32 said, as an unknown, that a PR might exist, because it didn't read
/// attachments. #35 reads them: with every attachment read and none a Pull
/// Request, den knows there is no PR, and the unknown is gone. (Incomplete
/// attachments withhold Implement instead: `decide_landing_groups.rs`.)
#[test]
fn a_linear_issue_whose_attachments_were_all_read_has_no_pr_unknown() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .build();
    let decisions = decide_all(&observation);
    let decision = decision_for(&decisions, "TEAM-1");
    assert_eq!(decision.next_step, Some(NextStep::Implement));
    assert!(decision.explanation.unknowns.is_empty(), "{decision:?}");
}

/// Story 16: identity is workspace + permanent ID. The same issue moved to
/// another team (new identifier) is the same Work Item; another workspace's
/// issue with the same ID is not.
#[test]
fn a_linear_issue_keeps_its_identity_when_it_moves_between_teams() {
    let before = ObservationBuilder::new()
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .build();
    let after = ObservationBuilder::new()
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-1", "OPS-7"))
        .build();
    let id_before = decision_for(&decide_all(&before), "TEAM-1").item.id.clone();
    let id_after = decision_for(&decide_all(&after), "OPS-7").item.id.clone();
    assert_eq!(id_before, id_after);
    assert!(same_work_item(&id_before, &id_after));
    assert_eq!(
        id_before,
        WorkItemId::LinearIssue {
            owner: "acme".to_owned(),
            issue: LinearIssueId::new("acme", "id-1"),
        }
    );
    assert_eq!(id_before.kind(), WorkItemKind::LinearIssue);

    let elsewhere = WorkItemId::LinearIssue {
        owner: "acme".to_owned(),
        issue: LinearIssueId::new("other-workspace", "id-1"),
    };
    assert!(!same_work_item(&id_before, &elsewhere));
}

/// A Linear Issue's coverage is its Owner's Linear sources; the GitHub
/// viewer doesn't bear on it, and its sources don't bear on GitHub items.
#[test]
fn linear_coverage_applies_to_linear_issues_only() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .linear_viewer("acme", "acme")
        .owner_coverage(
            "acme",
            SourceKind::Linear(LinearSourceKind::AssignedIssues),
            SourceState::Failed {
                error: "timed out".to_owned(),
            },
        )
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .pull_request(pr("acme/api", 4).author(ME))
        .build();
    let decisions = decide_all(&observation);
    let linear = decision_for(&decisions, "TEAM-1");
    assert!(
        linear.coverage.iter().all(|c| matches!(
            &c.source,
            Source::Owner {
                kind: SourceKind::Linear(_),
                ..
            }
        )),
        "{:?}",
        linear.coverage
    );
    assert!(linear.coverage.iter().any(|c| !c.state.is_complete()));
    let github = decisions
        .iter()
        .find(|d| d.item.id.kind() == WorkItemKind::PullRequest)
        .expect("the PR");
    assert!(
        github.coverage.iter().all(|c| c.state.is_complete()),
        "{:?}",
        github.coverage
    );
}

/// Linear Issues belong to their Owner: Owner and All Owners Scopes have
/// them; a Repo Scope has only those with a member there
/// (`decide_landing_groups.rs`), and this one has none.
#[test]
fn linear_issues_are_in_their_owners_scopes() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .build();
    let decisions = decide_all(&observation);
    let count = |scope: &Scope| in_scope(&decisions, scope).count();
    assert_eq!(count(&Scope::AllOwners), 1);
    assert_eq!(count(&Scope::Owner("ACME".to_owned())), 1);
    assert_eq!(count(&Scope::Owner("other".to_owned())), 0);
    assert_eq!(
        count(&Scope::Repo(den::domain::RepoId::new("acme", "api"))),
        0
    );
}

/// Linear Issues sit beside GitHub Work Items; they never change a GitHub
/// Work Item's Decision.
#[test]
fn linear_issues_leave_github_decisions_as_they_were() {
    let github = || {
        ObservationBuilder::new()
            .fully_read_owner("acme")
            .repo("acme/api")
            .pull_request(pr("acme/api", 1).author(ME).approved().passing("ci"))
            .issue(issue("acme/api", 2).label("bug"))
    };
    let without = decide_all(&github().build());
    let with = decide_all(
        &github()
            .linear_viewer("acme", "acme")
            .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
            .build(),
    );
    let github_only: Vec<&Decision> = with
        .iter()
        .filter(|d| d.item.id.kind() != WorkItemKind::LinearIssue)
        .collect();
    assert_eq!(github_only, without.iter().collect::<Vec<_>>());
    assert_eq!(with.len(), without.len() + 1);
}
