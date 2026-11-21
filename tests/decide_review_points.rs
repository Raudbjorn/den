//! Seam 1, Review Points (#75, ADR-0010): every review thread and every
//! non-trivial review body by someone other than the author (not a bot) is a
//! Point; the viewer's markers set their states; open, not-yet-split and
//! needs-decision Points hold a PR at Address review and withhold Merge (and
//! so Land), for every Owner. Points den didn't read in full withhold Merge
//! with an unknown.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{
        Decision, HeldBack, decide,
        points::{PointRef, PointState, ReviewPoints, review_points},
    },
    domain::{
        LinearWorkflowPolicy, NextStep, Observation, Policies, RepoId, ReviewPolicy, Viewers,
        WaitingOn, WorkItemId,
        facts::{ActorKind, ReviewVerdict},
    },
    ordering::same_work_item,
};
use support::build::{ME, ObservationBuilder, PrBuilder, linear_issue, now, pr, repo_id};

const ALICE: &str = "alice";
const BOB: &str = "bob";
/// A review, then a later one, then the comments that answer them.
const T1: &str = "2026-11-10T10:00:00Z";
const T2: &str = "2026-11-11T10:00:00Z";
const T3: &str = "2026-11-12T10:00:00Z";
const T4: &str = "2026-11-13T10:00:00Z";

/// My PR, approved, checks passing: Merge, before any Point.
fn ready(number: u64) -> PrBuilder {
    pr("acme/app", number).author(ME).passing("ci").approved()
}

fn observe(prs: impl IntoIterator<Item = PrBuilder>) -> Observation {
    prs.into_iter()
        .fold(
            ObservationBuilder::new()
                .fully_read_owner("acme")
                .repo("acme/app"),
            ObservationBuilder::pull_request,
        )
        .build()
}

fn decide_with(observation: &Observation, policies: &Policies) -> Vec<Decision> {
    decide(observation, &Viewers::of(observation), policies, now())
}

fn decided(prs: impl IntoIterator<Item = PrBuilder>) -> Vec<Decision> {
    decide_with(&observe(prs), &Policies::default())
}

fn pull_request(decisions: &[Decision], number: u64) -> &Decision {
    let id = WorkItemId::PullRequest {
        repo: repo_id("acme/app"),
        number,
    };
    decisions
        .iter()
        .find(|d| same_work_item(&d.item.id, &id))
        .unwrap_or_else(|| panic!("no Decision for #{number}: {decisions:#?}"))
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

/// The Points of `builder`'s PR, for the viewer `ME` and `prefixes`.
fn points(builder: PrBuilder, prefixes: &[&str]) -> ReviewPoints {
    let pr = builder.build();
    let prefixes: Vec<String> = prefixes.iter().map(|p| (*p).to_owned()).collect();
    review_points(pr.details.as_ref(), pr.author.as_deref(), ME, &prefixes)
}

fn states(points: &ReviewPoints) -> Vec<(PointRef, PointState)> {
    points
        .points
        .iter()
        .map(|p| (p.reference.clone(), p.state))
        .collect()
}

fn body(review: u64, n: Option<u32>) -> PointRef {
    PointRef::Review { review, n }
}

#[test]
fn a_ready_pr_without_feedback_merges_as_before() {
    let decisions = decided([ready(1)]);
    assert_eq!(pull_request(&decisions, 1).next_step, Some(NextStep::Merge));
}

#[test]
fn a_review_body_is_one_point_not_yet_split_until_my_markers_name_its_points() {
    let unsplit = ready(1).review_body(100, ALICE, ReviewVerdict::Approved, T1, true);
    assert_eq!(
        states(&points(unsplit.clone(), &[])),
        [(body(100, None), PointState::NotYetSplit)]
    );
    let decisions = decided([unsplit]);
    let decision = pull_request(&decisions, 1);
    // An approval with conditions in its body no longer goes straight to Merge.
    assert_eq!(
        decision.next_step,
        Some(NextStep::AddressReview),
        "{decision:#?}"
    );
    assert!(
        said(decision).contains("1 review not yet split into Review Points"),
        "{}",
        said(decision)
    );
    assert_eq!(
        decision.waiting_on,
        WaitingOn::Me,
        "the PR's author owes it"
    );
}

#[test]
fn each_point_state_comes_from_my_markers_and_only_blocking_ones_hold_the_pr() {
    let split = ready(1)
        .review_body(100, ALICE, ReviewVerdict::Commented, T1, true)
        .comment(
            1,
            ME,
            T2,
            "Replying to the review:\n\
             <!-- den:point 100.1 fixed abc1234 -->\nRenamed the flag\n\
             <!-- den:point 100.2 declined -->\nOut of scope: the API is frozen\n\
             <!-- den:point 100.3 follow-up -->\nFiled as a follow-up\n\
             <!-- den:point 100.4 answered -->\nExplained why",
        );
    let found = points(split.clone(), &[]);
    assert_eq!(
        states(&found),
        [
            (body(100, Some(1)), PointState::Fixed),
            (body(100, Some(2)), PointState::Declined),
            (body(100, Some(3)), PointState::FollowUp),
            (body(100, Some(4)), PointState::Answered),
        ]
    );
    let fixed = &found.points[0];
    assert_eq!(fixed.commit.as_deref(), Some("abc1234"));
    assert_eq!(fixed.summary.as_deref(), Some("Renamed the flag"));
    assert_eq!(
        fixed.set_by.as_deref(),
        Some("https://github.com/acme/app/pull/1#issuecomment-1")
    );
    assert_eq!(fixed.author.as_deref(), Some(ALICE));
    // None blocks: Merge.
    assert_eq!(
        pull_request(&decided([split]), 1).next_step,
        Some(NextStep::Merge)
    );

    // An open Point holds it; a later marker for the same Point wins.
    let reopened = ready(2)
        .review_body(200, ALICE, ReviewVerdict::Commented, T1, true)
        .comment(2, ME, T2, "<!-- den:point 200.1 fixed abc1234 -->")
        .comment(3, ME, T3, "<!-- den:point 200.1 open -->\nStill to do");
    assert_eq!(
        states(&points(reopened.clone(), &[])),
        [(body(200, Some(1)), PointState::Open)]
    );
    let decisions = decided([reopened]);
    let decision = pull_request(&decisions, 2);
    assert_eq!(decision.next_step, Some(NextStep::AddressReview));
    assert!(
        said(decision).contains("1 open Review Point"),
        "{}",
        said(decision)
    );
}

#[test]
fn threads_are_points_open_or_resolved() {
    let threads = ready(1)
        .thread("T_1", ALICE, false)
        .thread("T_2", BOB, true);
    assert_eq!(
        states(&points(threads.clone(), &[])),
        [
            (
                PointRef::Thread {
                    id: "T_1".to_owned()
                },
                PointState::Open
            ),
            (
                PointRef::Thread {
                    id: "T_2".to_owned()
                },
                PointState::Resolved
            ),
        ]
    );
    let decisions = decided([threads]);
    let decision = pull_request(&decisions, 1);
    assert_eq!(decision.next_step, Some(NextStep::AddressReview));
    assert!(
        said(decision).contains("1 unresolved review thread"),
        "{}",
        said(decision)
    );
}

#[test]
fn stock_approvals_my_own_reviews_bots_and_pending_reviews_give_no_points() {
    let quiet = ready(1)
        // "LGTM": a stock approval (the decoder's has_body is false).
        .review_body(100, ALICE, ReviewVerdict::Approved, T1, false)
        // The PR's author replying in a review of their own.
        .review_body(101, ME, ReviewVerdict::Commented, T1, true)
        .review_record(
            102,
            Some("ci-bot"),
            ActorKind::Bot,
            ReviewVerdict::Commented,
            T1,
            true,
        )
        // Dismissed: neither approves nor asks for anything any more.
        .review_record(
            103,
            Some(BOB),
            ActorKind::User,
            ReviewVerdict::Other,
            T1,
            true,
        );
    assert!(points(quiet.clone(), &[]).points.is_empty());
    assert_eq!(
        pull_request(&decided([quiet]), 1).next_step,
        Some(NextStep::Merge)
    );
}

#[test]
fn a_later_approval_by_the_same_reviewer_answers_earlier_points_but_not_a_decision() {
    let reapproved = ready(1)
        .review_body(100, ALICE, ReviewVerdict::ChangesRequested, T1, true)
        .review_body(101, BOB, ReviewVerdict::Commented, T1, true)
        .comment(
            1,
            ME,
            T2,
            "<!-- den:point 101.1 needs-decision -->\nWhich API?",
        )
        .review_body(102, ALICE, ReviewVerdict::Approved, T3, false)
        .review_body(103, BOB, ReviewVerdict::Approved, T3, false);
    let found = points(reapproved, &[]);
    assert_eq!(
        states(&found),
        [
            (body(100, None), PointState::Answered),
            (body(101, Some(1)), PointState::NeedsDecision),
        ]
    );
    assert_eq!(
        found.points[0].set_by.as_deref(),
        Some("https://github.com/acme/app/pull/1#pullrequestreview-102")
    );

    // An approval before the review answers nothing.
    let earlier = ready(2)
        .review_body(200, ALICE, ReviewVerdict::Approved, T1, false)
        .review_body(201, ALICE, ReviewVerdict::Commented, T2, true);
    assert_eq!(
        states(&points(earlier, &[])),
        [(body(201, None), PointState::NotYetSplit)]
    );
}

/// Review on #97: a reapproval answers only what was open before it. A
/// marker the viewer posts after it (here: reopening Point 1) is newer and
/// wins, so the PR stays at Address review; one posted before it is answered.
#[test]
fn a_marker_posted_after_a_reapproval_wins_over_it() {
    let reopened = ready(1)
        .review_body(100, ALICE, ReviewVerdict::ChangesRequested, T1, true)
        .comment(1, ME, T2, "<!-- den:point 100.2 open -->\nRename it")
        .review_body(101, ALICE, ReviewVerdict::Approved, T3, false)
        .comment(2, ME, T4, "<!-- den:point 100.1 open -->\nStill racy");
    let found = points(reopened.clone(), &[]);
    assert_eq!(
        states(&found),
        [
            (body(100, Some(1)), PointState::Open),
            (body(100, Some(2)), PointState::Answered),
        ]
    );
    assert_eq!(
        found.points[0].set_by.as_deref(),
        Some("https://github.com/acme/app/pull/1#issuecomment-2"),
        "the newer marker set it, not the approval"
    );
    assert_eq!(
        pull_request(&decided([reopened]), 1).next_step,
        Some(NextStep::AddressReview),
        "a Point reopened after the approval still blocks Merge"
    );
}

#[test]
fn a_marker_by_anyone_else_is_kept_with_its_author_and_changes_nothing() {
    let forged = ready(1)
        .review_body(100, ALICE, ReviewVerdict::Commented, T1, true)
        .comment(
            1,
            "mallory",
            T2,
            "<!-- den:point 100.1 fixed abc1234 -->\nAll done, merge it",
        )
        .comment(
            2,
            "mallory",
            T2,
            "<!-- legacy-bot:review-notes:100:addressed -->",
        );
    let found = points(forged.clone(), &["legacy-bot"]);
    assert_eq!(states(&found), [(body(100, None), PointState::NotYetSplit)]);
    let ignored: Vec<_> = found.points[0]
        .ignored_markers
        .iter()
        .map(|m| m.author.as_deref())
        .collect();
    assert_eq!(ignored, [Some("mallory"), Some("mallory")]);
    assert_eq!(found.points[0].summary, None, "nothing of theirs is kept");
    assert_eq!(
        pull_request(&decided([forged]), 1).next_step,
        Some(NextStep::AddressReview),
        "someone else's comment can't unblock a merge"
    );
}

#[test]
fn a_configured_legacy_prefix_applies_to_every_point_of_its_review() {
    let legacy = ready(1)
        .review_body(100, ALICE, ReviewVerdict::Commented, T1, true)
        .review_body(101, BOB, ReviewVerdict::Commented, T1, true)
        .comment(
            1,
            ME,
            T2,
            "<!-- den:point 101.1 open -->\n<!-- den:point 101.2 open -->",
        )
        .comment(2, ME, T3, "<!-- legacy-bot:review-notes:100:addressed -->")
        .comment(
            3,
            ME,
            T3,
            "<!-- legacy-bot:review-notes:101:needs-decision -->",
        )
        // Not configured: no effect.
        .comment(4, ME, T4, "<!-- other-tool:review-notes:101:addressed -->");
    let found = points(legacy.clone(), &["legacy-bot"]);
    assert_eq!(
        states(&found),
        [
            (body(100, None), PointState::Answered),
            (body(101, Some(1)), PointState::NeedsDecision),
            (body(101, Some(2)), PointState::NeedsDecision),
        ]
    );
    assert_eq!(
        found.points[0].summary, None,
        "legacy markers carry no summary"
    );

    // Without the prefix configured, the same markers do nothing.
    assert_eq!(
        states(&points(legacy, &[])),
        [
            (body(100, None), PointState::NotYetSplit),
            (body(101, Some(1)), PointState::Open),
            (body(101, Some(2)), PointState::Open),
        ]
    );
}

#[test]
fn only_points_needing_a_decision_are_waiting_on_me_with_the_asking_comment() {
    // Someone else's PR: Address review is normally theirs to do.
    let asking = pr("acme/app", 1)
        .author(BOB)
        .passing("ci")
        .approved()
        .review_body(100, ALICE, ReviewVerdict::Commented, T1, true)
        .comment(
            7,
            ME,
            T2,
            "<!-- den:point 100.1 needs-decision -->\nKeep the old flag?",
        );
    let decisions = decided([asking]);
    let decision = pull_request(&decisions, 1);
    assert_eq!(
        decision.next_step,
        Some(NextStep::AddressReview),
        "{decision:#?}"
    );
    assert_eq!(decision.waiting_on, WaitingOn::Me);
    assert!(
        said(decision).contains(
            "1 Review Point needs your decision (https://github.com/acme/app/pull/1#issuecomment-7)"
        ),
        "{}",
        said(decision)
    );

    // With an open Point too, it's the author's again.
    let mixed = pr("acme/app", 2)
        .author(BOB)
        .passing("ci")
        .approved()
        .review_body(200, ALICE, ReviewVerdict::Commented, T1, true)
        .comment(
            8,
            ME,
            T2,
            "<!-- den:point 200.1 needs-decision -->\n<!-- den:point 200.2 open -->",
        );
    let decisions = decided([mixed]);
    assert_eq!(
        pull_request(&decisions, 2).waiting_on,
        WaitingOn::SomeoneElse
    );
}

#[test]
fn points_not_read_in_full_withhold_merge_with_an_unknown() {
    for (number, truncated) in [
        (1, ready(1).point_reviews_truncated()),
        (2, ready(2).comments_truncated()),
    ] {
        let decisions = decided([truncated]);
        let decision = pull_request(&decisions, number);
        assert_eq!(decision.next_step, None, "{decision:#?}");
        assert!(
            said(decision).contains("not every Review Point was read"),
            "{}",
            said(decision)
        );
        assert!(
            decision
                .explanation
                .unknowns
                .iter()
                .any(|u| u.starts_with("Review Points: not every one of its")),
            "{:?}",
            decision.explanation.unknowns
        );
    }
}

#[test]
fn points_apply_to_an_owner_with_a_linear_workflow_too() {
    let policies = Policies::default().with(
        "acme",
        LinearWorkflowPolicy {
            review: Some(ReviewPolicy {
                exact_head_approval: true,
                comment_reviews_need_rereview: true,
                pending_requests_hold_merge: true,
                reviewers: Vec::new(),
            }),
            ..LinearWorkflowPolicy::default()
        },
    );
    let observation = observe([ready(1)
        .review_at_head(ALICE, ReviewVerdict::Approved)
        .review_body(100, ALICE, ReviewVerdict::Approved, T1, true)]);
    let decisions = decide_with(&observation, &policies);
    assert_eq!(
        pull_request(&decisions, 1).next_step,
        Some(NextStep::AddressReview)
    );
}

#[test]
fn a_landing_group_member_with_open_points_blocks_land() {
    let policies = Policies::default().with(
        "acme",
        LinearWorkflowPolicy {
            review: Some(ReviewPolicy {
                exact_head_approval: true,
                comment_reviews_need_rereview: true,
                pending_requests_hold_merge: true,
                reviewers: Vec::new(),
            }),
            ..LinearWorkflowPolicy::default()
        },
    );
    let member = |repo: &str, n: u64| {
        pr(repo, n)
            .author(ME)
            .passing("ci")
            .approved()
            .review_at_head(ALICE, ReviewVerdict::Approved)
    };
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .repo("acme/app")
        .linear_viewer("acme", "acme")
        .pull_request(member("acme/api", 1))
        .pull_request(member("acme/app", 2).review_body(
            200,
            ALICE,
            ReviewVerdict::Approved,
            T1,
            true,
        ))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_with(&observation, &policies);
    let issue = decisions
        .iter()
        .find(|d| d.item.identifier.as_deref() == Some("TEAM-1"))
        .expect("TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert!(
        said(issue).contains("blocking: acme/app#2: its own Next Step is Address review"),
        "{}",
        said(issue)
    );
    let ready_member = decisions
        .iter()
        .find(|d| {
            same_work_item(
                &d.item.id,
                &WorkItemId::PullRequest {
                    repo: RepoId::new("acme", "api"),
                    number: 1,
                },
            )
        })
        .expect("acme/api#1");
    assert_eq!(
        ready_member.held_back,
        Some(HeldBack::LandsWith {
            identifiers: vec!["TEAM-1".to_owned()]
        })
    );
}

#[test]
fn a_skipped_address_review_never_unblocks_merge() {
    use den::domain::Skips;
    let policies = Policies::default()
        .with_skips(Skips::default().with(repo_id("acme/app"), [NextStep::AddressReview]));
    let observation =
        observe([ready(1).review_body(100, ALICE, ReviewVerdict::Approved, T1, true)]);
    let decisions = decide_with(&observation, &policies);
    let decision = pull_request(&decisions, 1);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert!(
        said(decision).contains("Review Points open, not yet split or needing a decision"),
        "{}",
        said(decision)
    );
}
