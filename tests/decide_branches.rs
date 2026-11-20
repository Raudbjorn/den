//! Seam 1: the decision engine on Branches. Observation in, Decisions out.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide, decide_with},
    domain::{
        Incompleteness, NextStep, Observation, SourceKind, SourceState, UpstreamTargets, WaitingOn,
        WorkItemId,
    },
};
use jiff::SignedDuration;
use support::build::{ME, ObservationBuilder, branch, branch::fork, now, repo_id};

fn decisions(observation: &Observation) -> Vec<Decision> {
    decide(observation, &observation.viewer, now())
}

fn branch_id(repo: &str, name: &str) -> WorkItemId {
    WorkItemId::Branch {
        repo: repo_id(repo),
        name: name.to_owned(),
    }
}

fn find<'a>(decisions: &'a [Decision], id: &WorkItemId) -> Option<&'a Decision> {
    decisions.iter().find(|d| &d.item.id == id)
}

fn only(observation: &Observation) -> Decision {
    match decisions(observation).as_slice() {
        [one] => one.clone(),
        other => panic!("expected one Decision, got {other:#?}"),
    }
}

fn read(owner: &str) -> ObservationBuilder {
    ObservationBuilder::new()
        .fully_read_owner(owner)
        .repo(&format!("{owner}/app"))
}

#[test]
fn age_decides_between_open_pr_and_prune_or_revive() {
    let day = SignedDuration::from_hours(24);
    let cases = [
        ("29 days", day * 29, NextStep::OpenPr),
        (
            "exactly 30 days is still within",
            day * 30,
            NextStep::OpenPr,
        ),
        (
            "30 days and a second is stale",
            day * 30 + SignedDuration::from_secs(1),
            NextStep::PruneOrRevive,
        ),
        ("31 days", day * 31, NextStep::PruneOrRevive),
        (
            "a commit dated in the future counts as new",
            -day,
            NextStep::OpenPr,
        ),
    ];
    for (name, age, step) in cases {
        let observation = read("o")
            .branch(branch("o/app", "feature").age(age).author(ME))
            .build();
        let decision = only(&observation);
        assert_eq!(decision.next_step, Some(step), "{name}");
        assert_eq!(decision.waiting_on, WaitingOn::Me, "{name}");
        assert!(!decision.unattributed, "{name}");
        assert!(
            decision.waiting_reason.contains("last commit"),
            "{name}: {}",
            decision.waiting_reason
        );
    }
}

#[test]
fn the_explanation_gives_age_and_commits_ahead_and_frames_prune_as_a_decision() {
    let fresh = only(
        &read("o")
            .branch(branch("o/app", "fresh").age_days(29).ahead(3))
            .build(),
    );
    let rule = fresh
        .explanation
        .winning_rule
        .clone()
        .expect("a winning rule");
    for needle in [
        "3 commits not on main",
        "never the head of a PR",
        "29 days ago",
        "within 30",
    ] {
        assert!(rule.contains(needle), "{needle:?} in {rule:?}");
    }

    let stale = only(
        &read("o")
            .branch(branch("o/app", "old").age_days(31).ahead(1))
            .build(),
    );
    let rule = stale
        .explanation
        .winning_rule
        .clone()
        .expect("a winning rule");
    for needle in [
        "Stale Branch",
        "1 commit not on main",
        "31 days ago",
        "more than 30",
        "prune it or revive it",
    ] {
        assert!(rule.contains(needle), "{needle:?} in {rule:?}");
    }
    let also = stale.explanation.other_conditions.join("\n");
    assert!(also.contains("not proof the work is unshipped"), "{also}");
    assert!(also.contains("squash and rebase merges"), "{also}");
}

#[test]
fn the_default_branch_is_never_a_work_item() {
    let observation = read("o")
        .branch(branch("o/app", "main").age_days(90))
        .branch(branch("o/app", "feature"))
        .build();
    let decisions = decisions(&observation);
    assert!(find(&decisions, &branch_id("o/app", "main")).is_none());
    assert!(find(&decisions, &branch_id("o/app", "feature")).is_some());
}

#[test]
fn a_branch_that_was_ever_a_pr_head_is_not_a_work_item() {
    let observation = read("o")
        .branch(branch("o/app", "merged-long-ago").age_days(200).had_pr())
        .branch(
            branch("o/app", "recent-pr")
                .age_days(1)
                .had_pr()
                .not_compared(),
        )
        .build();
    assert_eq!(decisions(&observation), Vec::<Decision>::new());
}

#[test]
fn a_branch_with_nothing_ahead_is_not_a_work_item() {
    let observation = read("o")
        .branch(branch("o/app", "fast-forwarded").ahead(0))
        .build();
    assert_eq!(decisions(&observation), Vec::<Decision>::new());
}

#[test]
fn unknown_commits_ahead_gives_no_step_and_says_so() {
    let decision = only(
        &read("o")
            .branch(branch("o/app", "feature").not_compared())
            .build(),
    );
    assert_eq!(decision.next_step, None);
    assert_eq!(decision.waiting_on, WaitingOn::Nobody);
    assert!(!decision.unattributed);
    let why = decision
        .explanation
        .no_step_reason
        .expect("a no-step reason");
    assert!(why.contains("could not establish"), "{why}");
    assert!(
        decision
            .explanation
            .unknowns
            .iter()
            .any(|u| u.contains("commits ahead of main")),
        "{:?}",
        decision.explanation.unknowns
    );
}

#[test]
fn same_named_branches_in_two_repos_are_two_work_items() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("alpha")
        .fully_read_owner("beta")
        .repo("alpha/app")
        .repo("alpha/lib")
        .repo("beta/app")
        .branch(branch("alpha/app", "wip").age_days(2))
        .branch(branch("alpha/lib", "wip").age_days(40))
        .branch(branch("beta/app", "wip").age_days(3))
        .build();
    let decisions = decisions(&observation);
    assert_eq!(decisions.len(), 3);
    for (repo, step) in [
        ("alpha/app", NextStep::OpenPr),
        ("alpha/lib", NextStep::PruneOrRevive),
        ("beta/app", NextStep::OpenPr),
    ] {
        let decision = find(&decisions, &branch_id(repo, "wip")).expect(repo);
        assert_eq!(decision.next_step, Some(step), "{repo}");
        assert_eq!(decision.item.id.reference(), "wip");
    }
}

#[test]
fn a_forks_branch_is_judged_against_the_forks_own_default_branch() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("me")
        .with_repo(fork("me/tool", "upstream/tool", "trunk"))
        .branch(
            branch("me/tool", "my-fix")
                .compared_against("me/tool", "trunk")
                .ahead(2)
                .author(ME),
        )
        // The fork's own default branch is not a Work Item, even though the
        // parent's default is called something else.
        .branch(branch("me/tool", "trunk").compared_against("me/tool", "trunk"))
        .build();
    let decision = only(&observation);
    assert_eq!(decision.item.id, branch_id("me/tool", "my-fix"));
    assert_eq!(decision.next_step, Some(NextStep::OpenPr));
    let rule = decision.explanation.winning_rule.expect("a winning rule");
    assert!(rule.contains("2 commits not on trunk"), "{rule}");
    assert!(!rule.contains("upstream"), "{rule}");
}

/// Decisions with `forks` targeting their Upstream (#13).
fn decide_targeting(observation: &Observation, forks: &[&str]) -> Vec<Decision> {
    let upstreams = UpstreamTargets::new(forks.iter().map(|f| repo_id(f)));
    decide_with(observation, &observation.viewer, &upstreams, now())
}

/// A fork me/tool (own default `trunk`) of upstream/tool (default `main`),
/// with one Branch `my-fix` compared against `against` ("owner/name", branch).
fn fork_with_branch(against: (&str, &str)) -> Observation {
    ObservationBuilder::new()
        .fully_read_owner("me")
        .with_repo(fork("me/tool", "upstream/tool", "trunk"))
        .branch(
            branch("me/tool", "my-fix")
                .compared_against(against.0, against.1)
                .ahead(3)
                .author(ME),
        )
        .build()
}

#[test]
fn with_upstream_set_a_forks_branch_is_judged_against_the_upstreams_default_branch() {
    // The survey, told the fork targets its Upstream, compared it there
    // (`owner:branch` across Repos, see tests/github_branches.rs).
    // Before #13 this test decided with no Upstream set; a comparison against
    // the Upstream is now only right when the Upstream is the target, which
    // the next test pins.
    let observation = fork_with_branch(("upstream/tool", "main"));
    let [decision] = decide_targeting(&observation, &["me/tool"])
        .try_into()
        .expect("one Decision");
    assert_eq!(decision.next_step, Some(NextStep::OpenPr));
    let rule = decision.explanation.winning_rule.expect("a rule");
    assert!(
        rule.contains("3 commits not on upstream/tool:main"),
        "{rule}"
    );
}

#[test]
fn a_branch_compared_against_the_wrong_branch_for_its_target_waits_for_a_refresh() {
    // Upstream set, but the Branch was counted against the fork's own default
    // (before the switch): den won't judge it against a branch not chosen.
    let stale = fork_with_branch(("me/tool", "trunk"));
    let [decision] = decide_targeting(&stale, &["me/tool"])
        .try_into()
        .expect("one Decision");
    assert_eq!(decision.next_step, None);
    let reason = decision.explanation.no_step_reason.expect("a reason");
    assert!(
        reason.contains("counted against trunk")
            && reason.contains("its Upstream's default branch upstream/tool:main")
            && reason.contains("refresh (r)"),
        "{reason}"
    );
    assert!(!decision.explanation.unknowns.is_empty());

    // And the other way round: switched back to the fork, a Branch counted
    // against the Upstream waits too.
    let [back] = decide_targeting(&fork_with_branch(("upstream/tool", "main")), &[])
        .try_into()
        .expect("one Decision");
    assert_eq!(back.next_step, None);
    let reason = back.explanation.no_step_reason.expect("a reason");
    assert!(reason.contains("its own default branch trunk"), "{reason}");
}

#[test]
fn upstream_set_for_a_fork_whose_upstream_den_cannot_see_gives_no_next_step() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("me")
        .fork("me/tool", None)
        .branch(branch("me/tool", "my-fix").author(ME))
        .build();
    let [decision] = decide_targeting(&observation, &["me/tool"])
        .try_into()
        .expect("one Decision");
    assert_eq!(decision.next_step, None);
    let reason = decision.explanation.no_step_reason.expect("a reason");
    assert!(reason.contains("can't see the Upstream"), "{reason}");
    // Upstream set for a Repo that isn't a fork means nothing: judged as usual.
    let plain = read("me")
        .branch(branch("me/app", "wip").author(ME))
        .build();
    let [own] = decide_targeting(&plain, &["me/app"])
        .try_into()
        .expect("one Decision");
    assert_eq!(own.next_step, Some(NextStep::OpenPr));
}

#[test]
fn waiting_on_is_the_last_commits_author() {
    let mine = only(&read("o").branch(branch("o/app", "a").author("Me")).build());
    assert_eq!(mine.waiting_on, WaitingOn::Me);
    let theirs = only(
        &read("o")
            .branch(branch("o/app", "a").author("alice"))
            .build(),
    );
    assert_eq!(theirs.waiting_on, WaitingOn::SomeoneElse);
    assert!(
        theirs.waiting_reason.contains("alice"),
        "{}",
        theirs.waiting_reason
    );
}

#[test]
fn an_unmatched_commit_author_is_waiting_on_nobody_and_unattributed() {
    for age in [2, 60] {
        let decision = only(
            &read("o")
                .branch(branch("o/app", "anon").age_days(age).unmatched_author())
                .build(),
        );
        assert!(decision.next_step.is_some(), "it keeps its Next Step");
        assert_eq!(decision.waiting_on, WaitingOn::Nobody);
        assert!(decision.unattributed);
        assert!(
            decision
                .explanation
                .unknowns
                .iter()
                .any(|u| u.contains("unattributed")),
            "{:?}",
            decision.explanation.unknowns
        );
    }
}

#[test]
fn the_title_is_the_last_commits_headline_and_the_age_its_time() {
    let decision = only(
        &read("o")
            .branch(
                branch("o/app", "feature")
                    .headline("Teach the parser tabs")
                    .age_days(4),
            )
            .build(),
    );
    assert_eq!(decision.item.title, "Teach the parser tabs");
    assert_eq!(
        decision.item.updated_at,
        now() - SignedDuration::from_hours(24 * 4)
    );
    assert_eq!(decision.item.url, "https://github.com/o/app/tree/feature");
}

#[test]
fn partial_branch_coverage_is_carried_onto_that_repos_branches_only() {
    let partial = SourceState::Partial(Incompleteness::PaginationStopped {
        pages_read: 50,
        reason: "den reads at most 50 pages per list".to_owned(),
    });
    let observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .repo("o/big")
        .repo("o/small")
        .repo_coverage("o/big", SourceKind::Branches, partial.clone())
        .branch(branch("o/big", "x"))
        .branch(branch("o/small", "x"))
        .build();
    let decisions = decisions(&observation);
    let incomplete = |repo: &str| -> Vec<SourceState> {
        find(&decisions, &branch_id(repo, "x"))
            .expect(repo)
            .coverage
            .iter()
            .filter(|c| !c.state.is_complete())
            .map(|c| c.state.clone())
            .collect()
    };
    assert_eq!(incomplete("o/big"), vec![partial]);
    assert_eq!(incomplete("o/small"), Vec::<SourceState>::new());
}
