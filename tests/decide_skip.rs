//! Seam 1, per-Repo `skip` (spec 0003, "Not tracking a flow"): a Repo that
//! doesn't track a Next Step kind never gets it. A Pull Request falls through
//! to its next rule; nothing a skip removes ever unblocks another step; the
//! Explanation still says the skipped kind's condition held; a Landing Group
//! member still lands with its Linear Issue.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, HeldBack, decide},
    domain::{
        LinearWorkflowPolicy, NextStep, Observation, Policies, RepoId, ReviewPolicy, Skips,
        Viewers, WaitingOn, WorkItemId, WorkItemKind, facts::ReviewVerdict,
    },
    ordering::same_work_item,
};
use support::build::{
    ME, ObservationBuilder, PrBuilder, branch, issue, linear_issue, now, pr, repo_id, run,
};

const OTHER: &str = "someone";

fn skipping(repo: &str, steps: &[NextStep]) -> Policies {
    Policies::default().with_skips(Skips::default().with(repo_id(repo), steps.iter().copied()))
}

fn decide_all(observation: &Observation, policies: &Policies) -> Vec<Decision> {
    decide(observation, &Viewers::of(observation), policies, now())
}

fn pull_request<'d>(decisions: &'d [Decision], repo: &str, number: u64) -> &'d Decision {
    let id = WorkItemId::PullRequest {
        repo: repo_id(repo),
        number,
    };
    decisions
        .iter()
        .find(|d| same_work_item(&d.item.id, &id))
        .unwrap_or_else(|| panic!("no Decision for {repo}#{number}: {decisions:#?}"))
}

fn of_kind(decisions: &[Decision], kind: WorkItemKind) -> &Decision {
    decisions
        .iter()
        .find(|d| d.item.id.kind() == kind)
        .unwrap_or_else(|| panic!("no {kind:?} Decision: {decisions:#?}"))
}

/// Everything the Explanation says, in one string.
fn said(decision: &Decision) -> String {
    let e = &decision.explanation;
    format!(
        "{} | {} | {} | {}",
        e.winning_rule.as_deref().unwrap_or_default(),
        e.no_step_reason.as_deref().unwrap_or_default(),
        e.other_conditions.join(" | "),
        e.unknowns.join(" | ")
    )
}

fn assert_says(decision: &Decision, needles: &[&str]) {
    let text = said(decision);
    for needle in needles {
        assert!(text.contains(needle), "{needle:?} not in {text:?}");
    }
}

fn acme() -> ObservationBuilder {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/app")
        .repo("acme/api")
}

#[test]
fn a_skipped_kind_falls_through_to_the_next_rule() {
    let observation = acme()
        .pull_request(
            pr("acme/app", 1)
                .author(OTHER)
                .review_requested_from(ME)
                .failing_required("ci"),
        )
        .build();
    let tracked = decide_all(&observation, &Policies::default());
    assert_eq!(
        pull_request(&tracked, "acme/app", 1).next_step,
        Some(NextStep::FixCi)
    );

    let decisions = decide_all(&observation, &skipping("acme/app", &[NextStep::FixCi]));
    let decision = pull_request(&decisions, "acme/app", 1);
    assert_eq!(decision.next_step, Some(NextStep::Review), "{decision:#?}");
    assert_eq!(decision.waiting_on, WaitingOn::Me);
    assert_eq!(decision.not_tracked, None);
    assert_says(decision, &["Fix CI is not tracked in this Repo (skip): "]);
}

#[test]
fn skipping_fix_ci_never_makes_a_failing_pr_mergeable() {
    let observation = acme()
        .pull_request(
            pr("acme/app", 2)
                .author(ME)
                .approved()
                .failing_required("ci"),
        )
        .build();
    let decisions = decide_all(&observation, &skipping("acme/app", &[NextStep::FixCi]));
    let decision = pull_request(&decisions, "acme/app", 2);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.not_tracked, Some(NextStep::FixCi));
    assert_eq!(decision.waiting_on, WaitingOn::Nobody);
    assert_says(
        decision,
        &["not tracked in this Repo (skip): Fix CI", "checks failing"],
    );
}

#[test]
fn skipping_resolve_conflicts_never_makes_a_conflicting_pr_mergeable() {
    let observation = acme()
        .pull_request(
            pr("acme/app", 3)
                .author(ME)
                .approved()
                .passing("ci")
                .conflicting(),
        )
        .build();
    let decisions = decide_all(
        &observation,
        &skipping("acme/app", &[NextStep::ResolveConflicts]),
    );
    let decision = pull_request(&decisions, "acme/app", 3);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.not_tracked, Some(NextStep::ResolveConflicts));
    assert_says(
        decision,
        &["Resolve conflicts is not tracked", "conflicts with"],
    );
}

#[test]
fn two_skipped_kinds_are_both_noted_and_the_earliest_recorded() {
    let observation = acme()
        .pull_request(pr("acme/app", 4).author(ME).draft().conflicting())
        .build();
    let decisions = decide_all(
        &observation,
        &skipping(
            "acme/app",
            &[NextStep::ResolveConflicts, NextStep::FinishDraft],
        ),
    );
    let decision = pull_request(&decisions, "acme/app", 4);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.not_tracked, Some(NextStep::FinishDraft));
    assert_says(
        decision,
        &[
            "Finish draft is not tracked in this Repo (skip): the PR is a draft",
            "Resolve conflicts is not tracked in this Repo (skip): ",
        ],
    );
}

#[test]
fn skipping_merge_withholds_merge_and_says_why() {
    let observation = acme()
        .pull_request(pr("acme/app", 5).author(ME).approved().passing("ci"))
        .build();
    let decisions = decide_all(&observation, &skipping("acme/app", &[NextStep::Merge]));
    let decision = pull_request(&decisions, "acme/app", 5);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.not_tracked, Some(NextStep::Merge));
    assert_eq!(decision.waiting_on, WaitingOn::Nobody);
    assert_says(decision, &["Merge is not tracked in this Repo (skip): "]);
}

#[test]
fn another_repos_skips_do_not_apply() {
    let observation = acme()
        .pull_request(pr("acme/api", 6).author(ME).failing_required("ci"))
        .build();
    let decisions = decide_all(&observation, &skipping("acme/app", &[NextStep::FixCi]));
    let decision = pull_request(&decisions, "acme/api", 6);
    assert_eq!(decision.next_step, Some(NextStep::FixCi));
    assert_eq!(decision.not_tracked, None);
}

#[test]
fn repo_names_compare_without_case() {
    let observation = acme()
        .pull_request(pr("acme/app", 7).author(ME).failing_required("ci"))
        .build();
    let policies = Policies::default()
        .with_skips(Skips::default().with(RepoId::new("ACME", "App"), [NextStep::FixCi]));
    let decisions = decide_all(&observation, &policies);
    assert_eq!(pull_request(&decisions, "acme/app", 7).next_step, None);
}

/// acme's Linear Workflow as `tests/decide_land.rs` builds it, without Held
/// Repos, and `acme/app` not tracking Merge.
fn acme_workflow_skipping_merge_in_app() -> Policies {
    Policies::default()
        .with(
            "acme",
            LinearWorkflowPolicy {
                review: Some(ReviewPolicy {
                    exact_head_approval: true,
                    comment_reviews_need_rereview: true,
                    reviewers: Vec::new(),
                }),
                ..LinearWorkflowPolicy::default()
            },
        )
        .with_skips(Skips::default().with(repo_id("acme/app"), [NextStep::Merge]))
}

fn ready(repo: &str, number: u64) -> PrBuilder {
    pr(repo, number)
        .author(ME)
        .passing("ci")
        .approved()
        .review_at_head("alice", ReviewVerdict::Approved)
}

#[test]
fn a_member_in_a_repo_that_skips_merge_still_lands_with_its_issue() {
    let observation = acme()
        .linear_viewer("acme", "acme")
        .pull_request(ready("acme/api", 1))
        .pull_request(ready("acme/app", 2))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_all(&observation, &acme_workflow_skipping_merge_in_app());

    let member = pull_request(&decisions, "acme/app", 2);
    assert_eq!(
        member.held_back,
        Some(HeldBack::LandsWith {
            identifiers: vec!["TEAM-1".to_owned()]
        }),
        "{member:#?}"
    );
    assert_eq!(member.not_tracked, None);
    let issue = decisions
        .iter()
        .find(|d| d.item.identifier.as_deref() == Some("TEAM-1"))
        .expect("TEAM-1 decided");
    assert_eq!(issue.next_step, Some(NextStep::Land), "{issue:#?}");
}

#[test]
fn a_run_whose_fix_ci_is_skipped_has_no_next_step_and_says_why() {
    let observation = acme().run(run("acme/app", "ci.yml", 1).failed()).build();
    let tracked = decide_all(&observation, &Policies::default());
    assert_eq!(
        of_kind(&tracked, WorkItemKind::Run).next_step,
        Some(NextStep::FixCi)
    );

    let decisions = decide_all(&observation, &skipping("acme/app", &[NextStep::FixCi]));
    let decision = of_kind(&decisions, WorkItemKind::Run);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.not_tracked, Some(NextStep::FixCi));
    assert_eq!(decision.waiting_on, WaitingOn::Nobody);
    assert!(!decision.unattributed);
    assert_says(decision, &["Fix CI is not tracked in this Repo (skip): "]);
    assert_eq!(decision.explanation.winning_rule, None);
}

#[test]
fn a_skipped_triage_withholds_the_issue() {
    let observation = acme().issue(issue("acme/app", 8)).build();
    let decisions = decide_all(&observation, &skipping("acme/app", &[NextStep::Triage]));
    let decision = of_kind(&decisions, WorkItemKind::Issue);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.not_tracked, Some(NextStep::Triage));
}

#[test]
fn a_skipped_prune_or_revive_never_turns_into_open_pr() {
    let observation = acme()
        .branch(branch("acme/app", "old").age_days(31).ahead(1).author(ME))
        .build();
    let tracked = decide_all(&observation, &Policies::default());
    assert_eq!(
        of_kind(&tracked, WorkItemKind::Branch).next_step,
        Some(NextStep::PruneOrRevive)
    );

    let decisions = decide_all(
        &observation,
        &skipping("acme/app", &[NextStep::PruneOrRevive]),
    );
    let decision = of_kind(&decisions, WorkItemKind::Branch);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.not_tracked, Some(NextStep::PruneOrRevive));
}

#[test]
fn a_linear_issues_implement_is_never_withheld_by_a_repo_skip() {
    let observation = acme()
        .linear_viewer("acme", "acme")
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .build();
    let decisions = decide_all(&observation, &skipping("acme/app", &[NextStep::Implement]));
    let decision = of_kind(&decisions, WorkItemKind::LinearIssue);
    assert_eq!(
        decision.next_step,
        Some(NextStep::Implement),
        "{decision:#?}"
    );
    assert_eq!(decision.not_tracked, None);
}
