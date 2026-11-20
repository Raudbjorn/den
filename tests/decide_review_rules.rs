//! Seam 1, the Linear Workflow's review rules for PRs (#36): fresh approval
//! at the current head, reviewers awaiting re-review, Address review only for
//! changes requested at the current head, Request review between Review and
//! Merge, Held Repos, the suggested-reviewers set, and the same facts under an
//! Owner without a Linear Workflow giving milestone 1's Decisions.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, HeldBack, decide},
    domain::{
        LinearWorkflowPolicy, NextStep, Observation, Policies, ReviewPolicy, SourceState, TeamRef,
        Viewers, WaitingOn,
        facts::{MergeState, ReviewDecision, ReviewVerdict, Reviewer},
    },
};
use support::build::{ME, ObservationBuilder, PrBuilder, now, pr, repo_id};

const OLDER_HEAD: &str = "0ld0000000000000000000000000000000000000";
const NEWER_HEAD: &str = "9e0000000000000000000000000000000000000f";
const EXACT_HEAD: &str = "workflow policy: approval must be at the current head";

struct Switches {
    exact_head: bool,
    comments: bool,
}

const BOTH_ON: Switches = Switches {
    exact_head: true,
    comments: true,
};

fn workflow(switches: Switches, reviewers: Vec<Reviewer>, held: &[&str]) -> LinearWorkflowPolicy {
    LinearWorkflowPolicy {
        review: Some(ReviewPolicy {
            exact_head_approval: switches.exact_head,
            comment_reviews_need_rereview: switches.comments,
            reviewers,
        }),
        held_repos: held.iter().map(|r| repo_id(r)).collect(),
        ..LinearWorkflowPolicy::default()
    }
}

/// `acme` with a Linear Workflow (both switches on unless given), Held Repo
/// `acme/app-held`.
fn acme(switches: Switches) -> Policies {
    Policies::default().with("acme", workflow(switches, Vec::new(), &["acme/app-held"]))
}

fn observation(prs: Vec<PrBuilder>) -> Observation {
    let mut builder = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .repo("acme/app-held")
        .fully_read_owner("octo")
        .repo("octo/api");
    for pr in prs {
        builder = builder.pull_request(pr);
    }
    builder.build()
}

fn decide_with(observation: &Observation, policies: &Policies) -> Vec<Decision> {
    decide(observation, &Viewers::of(observation), policies, now())
}

/// The Decision for the one PR `pr` with `policies`.
fn decided(pr: PrBuilder, policies: &Policies) -> Decision {
    let decisions = decide_with(&observation(vec![pr]), policies);
    assert_eq!(decisions.len(), 1, "{decisions:#?}");
    decisions[0].clone()
}

fn why(decision: &Decision) -> String {
    let e = &decision.explanation;
    e.winning_rule
        .clone()
        .or_else(|| e.no_step_reason.clone())
        .unwrap_or_default()
}

fn assert_says(decision: &Decision, needles: &[&str]) {
    let text = format!(
        "{} | {} | {}",
        why(decision),
        decision.explanation.other_conditions.join(" | "),
        decision.explanation.unknowns.join(" | ")
    );
    for needle in needles {
        assert!(text.contains(needle), "{needle:?} not in {text:?}");
    }
}

/// A ready PR by me in `acme/api`: passing, mergeable, GitHub says approved.
fn ready(number: u64) -> PrBuilder {
    pr("acme/api", number).author(ME).passing("ci").approved()
}

#[test]
fn a_draft_with_conflicts_in_a_workflow_owner_is_finish_draft_with_conflicts_explained() {
    let decision = decided(
        ready(1)
            .draft()
            .conflicting()
            .review_at_head("alice", ReviewVerdict::Approved),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, Some(NextStep::FinishDraft));
    assert_says(&decision, &["the PR is a draft", "conflicts with main"]);
}

#[test]
fn an_approval_at_an_older_head_is_request_review_with_exact_head_on_and_merge_with_it_off() {
    let pr = || ready(2).review_on("alice", ReviewVerdict::Approved, OLDER_HEAD);

    let on = decided(pr(), &acme(BOTH_ON));
    assert_eq!(on.next_step, Some(NextStep::RequestReview), "{on:#?}");
    assert_eq!(
        on.waiting_on,
        WaitingOn::Me,
        "Request review waits on the author"
    );
    assert!(on.waiting_reason.contains("the PR author"), "{on:?}");
    assert_says(&on, &["alice approved an older head", EXACT_HEAD]);

    let off = decided(
        pr(),
        &acme(Switches {
            exact_head: false,
            comments: true,
        }),
    );
    assert_eq!(off.next_step, Some(NextStep::Merge), "{off:#?}");
    assert_says(&off, &["approved", "nobody awaiting re-review"]);
}

#[test]
fn a_bot_approval_does_not_count() {
    let bot_only = decided(
        ready(3).bot_review_at_head("ci-bot", ReviewVerdict::Approved),
        &acme(BOTH_ON),
    );
    assert_eq!(bot_only.next_step, Some(NextStep::RequestReview));
    assert_says(
        &bot_only,
        &[
            "ci-bot is a bot",
            "workflow policy: bots' approvals don't count",
        ],
    );

    let with_a_user = decided(
        ready(3)
            .bot_review_at_head("ci-bot", ReviewVerdict::Approved)
            .review_at_head("alice", ReviewVerdict::Approved),
        &acme(BOTH_ON),
    );
    assert_eq!(with_a_user.next_step, Some(NextStep::Merge));
    assert_says(
        &with_a_user,
        &["approved at the current head by alice", EXACT_HEAD],
    );
}

#[test]
fn an_approval_with_a_pending_request_to_a_commented_reviewer_is_review_never_merge() {
    for switches in [
        BOTH_ON,
        Switches {
            exact_head: true,
            comments: false,
        },
    ] {
        let decision = decided(
            ready(4)
                .review_at_head("alice", ReviewVerdict::Approved)
                .review_on("bob", ReviewVerdict::Commented, OLDER_HEAD)
                .review_requested_from("bob"),
            &acme(switches),
        );
        assert_eq!(decision.next_step, Some(NextStep::Review), "{decision:#?}");
        assert_says(&decision, &["review requested from bob"]);
    }
}

#[test]
fn a_plain_comment_on_an_older_head_needs_re_review_only_when_the_comment_policy_is_on() {
    let pr = || {
        ready(5)
            .review_at_head("alice", ReviewVerdict::Approved)
            .review_on("bob", ReviewVerdict::Commented, OLDER_HEAD)
    };

    let on = decided(pr(), &acme(BOTH_ON));
    assert_eq!(on.next_step, Some(NextStep::RequestReview), "{on:#?}");
    assert_says(
        &on,
        &[
            "awaiting re-review: bob commented on an older head",
            "workflow policy: comment reviews need re-review",
        ],
    );

    let off = decided(
        pr(),
        &acme(Switches {
            exact_head: true,
            comments: false,
        }),
    );
    assert_eq!(off.next_step, Some(NextStep::Merge), "{off:#?}");
}

#[test]
fn a_comment_at_the_current_head_needs_no_re_review() {
    let decision = decided(
        ready(6)
            .review_at_head("alice", ReviewVerdict::Approved)
            .review_at_head("bob", ReviewVerdict::Commented),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, Some(NextStep::Merge), "{decision:#?}");
}

#[test]
fn a_comment_after_an_approval_at_the_same_head_keeps_the_approval() {
    // GitHub lists alice's latest review as COMMENTED, but her approval at
    // this head still stands.
    let decision = decided(
        ready(7)
            .review_at_head("alice", ReviewVerdict::Approved)
            .review_at_head("alice", ReviewVerdict::Commented),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, Some(NextStep::Merge), "{decision:#?}");
}

#[test]
fn changes_requested_at_the_current_head_is_address_review() {
    let decision = decided(
        ready(8)
            .changes_requested()
            .review_at_head("bob", ReviewVerdict::ChangesRequested),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, Some(NextStep::AddressReview));
    assert_says(
        &decision,
        &[
            "changes requested at the current head by bob",
            "workflow policy: only changes requested at the current head",
        ],
    );
}

#[test]
fn changes_requested_on_an_older_head_with_no_threads_falls_through_to_request_review() {
    let decision = decided(
        ready(9)
            .changes_requested()
            .review_on("bob", ReviewVerdict::ChangesRequested, OLDER_HEAD),
        &acme(BOTH_ON),
    );
    assert_eq!(
        decision.next_step,
        Some(NextStep::RequestReview),
        "{decision:#?}"
    );
    assert_says(
        &decision,
        &[
            "bob requested changes on an older head",
            "workflow policy: feedback given on an older head needs a fresh look",
        ],
    );

    // Unresolved threads still need addressing, whichever head.
    let with_threads = decided(
        ready(9)
            .changes_requested()
            .review_on("bob", ReviewVerdict::ChangesRequested, OLDER_HEAD)
            .unresolved_threads(1),
        &acme(BOTH_ON),
    );
    assert_eq!(with_threads.next_step, Some(NextStep::AddressReview));
    assert_says(&with_threads, &["1 unresolved review thread"]);
}

#[test]
fn someone_awaiting_re_review_blocks_merge_even_with_a_fresh_approval() {
    let decision = decided(
        ready(10)
            .review_at_head("alice", ReviewVerdict::Approved)
            .review_on("bob", ReviewVerdict::ChangesRequested, OLDER_HEAD),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, Some(NextStep::RequestReview));
    assert_says(&decision, &["awaiting re-review: bob requested changes"]);
}

#[test]
fn the_author_replying_on_an_older_head_is_not_awaiting_re_review() {
    let decision = decided(
        ready(11)
            .review_at_head("alice", ReviewVerdict::Approved)
            .review_on(ME, ReviewVerdict::Commented, OLDER_HEAD),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, Some(NextStep::Merge), "{decision:#?}");
}

#[test]
fn a_fresh_approval_github_still_wants_more_review_for_is_request_review() {
    // Branch protection wants a second approval (or a code owner's): GitHub's
    // decision stays REVIEW_REQUIRED and its merge state BLOCKED. The
    // workflow's switches tighten approval; they don't overrule GitHub.
    let decision = decided(
        ready(17)
            .decision(ReviewDecision::ReviewRequired)
            .merge_state(MergeState::Blocked)
            .review_at_head("alice", ReviewVerdict::Approved),
        &acme(BOTH_ON),
    );
    assert_eq!(
        decision.next_step,
        Some(NextStep::RequestReview),
        "{decision:#?}"
    );
    assert_says(
        &decision,
        &["GitHub reports that a review is still required"],
    );

    // GitHub's decision unread (a response with errors): not known approved.
    let unread = decided(
        ready(17)
            .decision(ReviewDecision::Unknown)
            .review_at_head("alice", ReviewVerdict::Approved),
        &acme(BOTH_ON),
    );
    assert_ne!(unread.next_step, Some(NextStep::Merge), "{unread:#?}");
    assert_says(&unread, &["approval could not be established"]);
}

#[test]
fn changes_requested_at_the_current_head_also_stop_merge() {
    // No branch protection, so GitHub gives no decision and reports CLEAN.
    let decision = decided(
        ready(18)
            .no_review_required()
            .review_at_head("alice", ReviewVerdict::Approved)
            .review_at_head("bob", ReviewVerdict::ChangesRequested),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, Some(NextStep::AddressReview));
    let merge_holds = decision
        .explanation
        .other_conditions
        .iter()
        .any(|c| c.contains("mergeable into"));
    assert!(!merge_holds, "Merge must not hold: {decision:#?}");
}

#[test]
fn a_reviewer_who_since_commented_on_the_current_head_is_not_awaiting_re_review() {
    // bob asked for changes, the author pushed, and bob commented on the new
    // head without approving: he has looked, so asking him again is redundant.
    let pr = || {
        ready(19)
            .review_at_head("alice", ReviewVerdict::Approved)
            .review_on("bob", ReviewVerdict::ChangesRequested, OLDER_HEAD)
            .review_at_head("bob", ReviewVerdict::Commented)
    };
    let policies = Policies::default().with(
        "acme",
        workflow(BOTH_ON, vec![Reviewer::User("carol".to_owned())], &[]),
    );

    // No branch protection: his older change request no longer stops Merge.
    let unprotected = decided(pr().no_review_required(), &policies);
    assert_eq!(
        unprotected.next_step,
        Some(NextStep::Merge),
        "{unprotected:#?}"
    );
    assert_says(&unprotected, &["changes requested on an older head by bob"]);
    assert_eq!(
        unprotected.suggested_reviewers,
        Some(vec![Reviewer::User("carol".to_owned())])
    );

    // Branch protection: GitHub still counts his change request, and says so.
    let protected = decided(pr().changes_requested(), &policies);
    assert_eq!(protected.next_step, Some(NextStep::RequestReview));
    assert_says(&protected, &["GitHub reports changes requested"]);
    for decision in [&unprotected, &protected] {
        let text = format!("{} {:?}", why(decision), decision.explanation);
        assert!(!text.contains("awaiting re-review: bob"), "{text}");
    }
}

#[test]
fn reviews_read_after_a_push_are_not_judged_against_the_listed_head() {
    // The PR list read the head 0x14; someone pushed before the details
    // query, which read the new head with alice's approval of 0x14 still
    // standing. That approval must not count as fresh.
    let decision = decided(
        ready(20)
            .review_at_head("alice", ReviewVerdict::Approved)
            .details_read_at(NEWER_HEAD),
        &acme(BOTH_ON),
    );
    assert_ne!(decision.next_step, Some(NextStep::Merge), "{decision:#?}");
    assert_says(
        &decision,
        &["the PR's head moved while den read its reviews"],
    );
}

#[test]
fn reviews_den_could_not_read_never_give_merge() {
    let unread = decided(ready(12).details_unread(), &acme(BOTH_ON));
    assert_ne!(unread.next_step, Some(NextStep::Merge));
    assert_ne!(unread.next_step, Some(NextStep::RequestReview));

    let truncated = decided(
        ready(12)
            .review_at_head("alice", ReviewVerdict::Approved)
            .reviews_truncated(),
        &acme(BOTH_ON),
    );
    assert_eq!(truncated.next_step, None, "{truncated:#?}");
    assert_says(&truncated, &["someone may be awaiting re-review"]);
}

#[test]
fn a_ready_pr_in_a_held_repo_has_no_next_step_and_is_held_for_release() {
    let decision = decided(
        pr("acme/app-held", 13)
            .author(ME)
            .passing("ci")
            .approved()
            .review_at_head("alice", ReviewVerdict::Approved),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.held_back, Some(HeldBack::ForRelease));
    assert_eq!(decision.waiting_on, WaitingOn::Nobody);
    assert_says(
        &decision,
        &[
            "held for release",
            "acme/app-held is a Held Repo (workflow policy",
        ],
    );
}

#[test]
fn a_held_pr_keeps_its_repair_steps() {
    let decision = decided(
        pr("acme/app-held", 14)
            .author(ME)
            .passing("ci")
            .approved()
            .conflicting()
            .review_at_head("alice", ReviewVerdict::Approved),
        &acme(BOTH_ON),
    );
    assert_eq!(decision.next_step, Some(NextStep::ResolveConflicts));
    assert_eq!(decision.held_back, None);

    let needs_review = decided(
        pr("acme/app-held", 15).author(ME).passing("ci"),
        &acme(BOTH_ON),
    );
    assert_eq!(needs_review.next_step, Some(NextStep::RequestReview));
}

#[test]
fn suggested_reviewers_are_configured_plus_awaiting_minus_approvers_requests_and_author() {
    let team = |slug: &str| {
        Reviewer::Team(TeamRef {
            org: "acme".to_owned(),
            slug: slug.to_owned(),
        })
    };
    let user = |login: &str| Reviewer::User(login.to_owned());
    let policies = Policies::default().with(
        "acme",
        workflow(
            BOTH_ON,
            vec![
                user("alice"),   // fresh approver: out
                user("Bob"),     // requested (as "bob"): out
                team("backend"), // in
                team("web"),     // requested: out
                user(ME),        // the author: out
                user("erin"),    // in
                user("ERIN"),    // the same person: once
            ],
            &[],
        ),
    );
    let decision = decided(
        ready(16)
            .review_at_head("alice", ReviewVerdict::Approved)
            .review_on("dave", ReviewVerdict::ChangesRequested, OLDER_HEAD) // awaiting: in
            .review_on("frank", ReviewVerdict::ChangesRequested, OLDER_HEAD) // asked again: out
            .review_requested_from("bob")
            .review_requested_from("frank")
            .review_requested_from_team("acme", "web"),
        &policies,
    );
    assert_eq!(
        decision.suggested_reviewers,
        Some(vec![team("backend"), user("erin"), user("dave")])
    );
}

#[test]
fn suggested_reviewers_say_when_den_could_not_filter_them_in_full() {
    let user = |login: &str| Reviewer::User(login.to_owned());
    let policies = Policies::default().with(
        "acme",
        workflow(BOTH_ON, vec![user("alice"), user("bob")], &[]),
    );
    // alice approved at the head, so she is out once den reads that.
    let pr = || ready(21).review_at_head("alice", ReviewVerdict::Approved);

    let exact = decided(pr(), &policies);
    assert_eq!(exact.suggested_reviewers, Some(vec![user("bob")]));
    assert_eq!(exact.suggested_reviewers_unread, None);

    // Nothing read: only the author is taken out, and the set says so.
    let unread = decided(pr().details_unread(), &policies);
    assert_eq!(
        unread.suggested_reviewers,
        Some(vec![user("alice"), user("bob")])
    );
    assert_eq!(
        unread.suggested_reviewers_unread.as_deref(),
        Some("the review requests and reviews could not be read")
    );

    let cases = [
        (
            pr().review_requests_truncated(),
            "not every review request was read",
        ),
        (pr().reviews_truncated(), "not every review was read"),
        (
            pr().details_read_at(NEWER_HEAD),
            "the PR's head moved while den read its reviews",
        ),
    ];
    for (pr, unread) in cases {
        let decision = decided(pr, &policies);
        assert_eq!(
            decision.suggested_reviewers_unread.as_deref(),
            Some(unread),
            "{decision:#?}"
        );
    }
}

#[test]
fn the_same_facts_under_an_owner_without_a_workflow_give_milestone_1_decisions() {
    // The same PRs under `acme` (with a workflow) and `octo` (without one).
    let deck = |owner: &str| -> Vec<PrBuilder> {
        let api = format!("{owner}/api");
        vec![
            pr(&api, 1).author(ME).passing("ci").approved().review_on(
                "alice",
                ReviewVerdict::Approved,
                OLDER_HEAD,
            ),
            pr(&api, 2)
                .author(ME)
                .passing("ci")
                .changes_requested()
                .review_on("bob", ReviewVerdict::ChangesRequested, OLDER_HEAD),
            pr(&api, 3)
                .author(ME)
                .passing("ci")
                .approved()
                .bot_review_at_head("ci-bot", ReviewVerdict::Approved),
            pr(&api, 4)
                .author(ME)
                .passing("ci")
                .approved()
                .review_at_head("alice", ReviewVerdict::Approved)
                .review_on("bob", ReviewVerdict::Commented, OLDER_HEAD),
            pr(&api, 5).author(ME).draft().conflicting(),
        ]
    };
    let mut prs = deck("acme");
    prs.extend(deck("octo"));
    let observation = observation(prs);
    let policies = acme(BOTH_ON).with(
        "octo-other",
        workflow(BOTH_ON, vec![Reviewer::User("x".to_owned())], &["octo/api"]),
    );

    let without = decide_with(&observation, &Policies::default());
    let with = decide_with(&observation, &policies);
    let octo = |decisions: &[Decision]| -> Vec<Decision> {
        decisions
            .iter()
            .filter(|d| d.item.id.owner() == "octo")
            .cloned()
            .collect()
    };
    // octo has no workflow: its Decisions don't depend on acme's.
    assert_eq!(octo(&with), octo(&without));
    // And they are milestone 1's: GitHub's review decision, no Request review.
    let steps: Vec<Option<NextStep>> = octo(&with).iter().map(|d| d.next_step).collect();
    assert_eq!(
        steps,
        [
            Some(NextStep::Merge),
            Some(NextStep::AddressReview),
            Some(NextStep::Merge),
            Some(NextStep::Merge),
            Some(NextStep::FinishDraft),
        ]
    );
    for decision in octo(&with) {
        assert_eq!(decision.suggested_reviewers, None);
        assert_eq!(decision.suggested_reviewers_unread, None);
        assert_eq!(decision.held_back, None);
    }
    // Under acme's workflow the same facts decide differently.
    let acme_steps: Vec<Option<NextStep>> = with
        .iter()
        .filter(|d| d.item.id.owner() == "acme")
        .map(|d| d.next_step)
        .collect();
    assert_eq!(
        acme_steps,
        [
            Some(NextStep::RequestReview),
            Some(NextStep::RequestReview),
            Some(NextStep::RequestReview),
            Some(NextStep::RequestReview),
            Some(NextStep::FinishDraft),
        ]
    );
}

/// A ready PR by me: alice approved the current head, bob only commented on
/// an older one.
fn approved_and_commented(owner: &str, number: u64) -> PrBuilder {
    pr(&format!("{owner}/api"), number)
        .author(ME)
        .passing("ci")
        .approved()
        .review_at_head("alice", ReviewVerdict::Approved)
        .review_on("bob", ReviewVerdict::Commented, OLDER_HEAD)
}

/// As [`approved_and_commented`], and GitHub listed more latest reviews of
/// any kind (`latestReviews`) than den read. The standing verdicts were read
/// in full.
fn latest_reviews_cut_short(owner: &str, number: u64) -> PrBuilder {
    approved_and_commented(owner, number).latest_reviews_truncated()
}

#[test]
fn truncated_latest_reviews_leave_an_owner_without_a_workflow_on_milestone_1() {
    // Only the workflow's rules read `latestReviews` (#51 review): an Owner
    // without one gets exactly the Decision it gets when the list was read in
    // full, with complete coverage.
    let truncated = decided(latest_reviews_cut_short("octo", 30), &acme(BOTH_ON));
    let milestone_1 = decided(approved_and_commented("octo", 30), &acme(BOTH_ON));

    assert_eq!(truncated, milestone_1);
    assert_eq!(truncated.next_step, Some(NextStep::Merge), "{truncated:#?}");
    assert!(truncated.explanation.unknowns.is_empty(), "{truncated:#?}");
    assert!(!truncated.coverage.is_empty(), "{truncated:#?}");
    assert!(
        truncated
            .coverage
            .iter()
            .all(|entry| entry.state == SourceState::Complete),
        "{truncated:#?}"
    );
}

#[test]
fn truncated_latest_reviews_under_a_workflow_are_unknown_and_never_give_merge() {
    // alice's approval at the head stands and the standing verdicts were read
    // in full, but someone den didn't read may have commented on an older
    // head: whether anyone awaits re-review is unknown, so no Merge.
    let decision = decided(
        ready(31)
            .review_at_head("alice", ReviewVerdict::Approved)
            .latest_reviews_truncated(),
        &acme(BOTH_ON),
    );
    assert_ne!(decision.next_step, Some(NextStep::Merge), "{decision:#?}");
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_says(
        &decision,
        &["not every review was read, so someone may be awaiting re-review"],
    );
    assert_eq!(
        decision.suggested_reviewers_unread.as_deref(),
        Some("not every review was read")
    );
}
