//! Seam 1: the decision engine on Pull Requests. Observation in, Decisions out.
//!
//! The case deck covers every PR rule, each adjacent precedence pair, what
//! never yields Merge, and how unknown facts are reported.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        Incompleteness, NextStep, Observation, Source, SourceKind, SourceState, WaitingOn,
        WorkItemId,
        facts::{CheckOutcome, MergeState, ReviewDecision, ReviewVerdict},
    },
};
use support::build::{ME, ObservationBuilder, PrBuilder, now, pr, repo_id};

fn decisions(observation: &Observation) -> Vec<Decision> {
    decide(observation, &observation.viewer, now())
}

fn pr_id(repo: &str, number: u64) -> WorkItemId {
    WorkItemId::PullRequest {
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

struct Expect {
    step: Option<NextStep>,
    waiting_on: WaitingOn,
    unattributed: bool,
    /// Substring of the winning rule (with a step) or of the no-step reason.
    why: &'static str,
    /// Each must be a substring of some other condition ("also: …").
    also: &'static [&'static str],
    /// Each must be a substring of some unknown.
    unknown: &'static [&'static str],
    /// Substring of the Waiting On reason.
    waiting_reason: &'static str,
}

const NONE: &[&str] = &[];

fn expect(
    step: Option<NextStep>,
    waiting_on: WaitingOn,
    why: &'static str,
    waiting_reason: &'static str,
) -> Expect {
    Expect {
        step,
        waiting_on,
        unattributed: false,
        why,
        also: NONE,
        unknown: NONE,
        waiting_reason,
    }
}

impl Expect {
    fn also(mut self, also: &'static [&'static str]) -> Self {
        self.also = also;
        self
    }

    fn unknown(mut self, unknown: &'static [&'static str]) -> Self {
        self.unknown = unknown;
        self
    }

    fn unattributed(mut self) -> Self {
        self.unattributed = true;
        self
    }
}

/// A PR that Merge holds for: approved, its check passing, mergeable.
fn ready(number: u64) -> PrBuilder {
    pr("o/r", number).approved().passing("ci / build")
}

/// Each case is one PR in "o/r", decided for viewer "me" (member of team
/// octo/core).
fn pr_rule_deck() -> Vec<(&'static str, PrBuilder, Expect)> {
    use NextStep::*;
    use WaitingOn::{Me, Nobody, SomeoneElse};
    vec![
        // ── Each rule on its own ─────────────────────────────────────────
        (
            "my draft",
            pr("o/r", 1).draft().author(ME),
            expect(Some(FinishDraft), Me, "draft", "you are the PR author"),
        ),
        (
            "my draft, login in another case",
            pr("o/r", 1).draft().author("ME"),
            expect(Some(FinishDraft), Me, "draft", "you are the PR author"),
        ),
        (
            "someone else's draft",
            pr("o/r", 1).draft().author("alice"),
            expect(
                Some(FinishDraft),
                SomeoneElse,
                "draft",
                "the PR author is alice",
            ),
        ),
        (
            "a draft whose author is gone",
            pr("o/r", 1).draft().no_author(),
            expect(
                Some(FinishDraft),
                Nobody,
                "draft",
                "could not establish the PR author",
            )
            .unattributed(),
        ),
        (
            "conflicting",
            pr("o/r", 1).author("alice").conflicting(),
            expect(
                Some(ResolveConflicts),
                SomeoneElse,
                "conflicts with main",
                "PR author is alice",
            ),
        ),
        (
            "a failing required check",
            pr("o/r", 1)
                .author(ME)
                .failing_required("ci / build")
                .passing("ci / lint"),
            expect(
                Some(FixCi),
                Me,
                "1 failing required check: ci / build (required)",
                "PR author",
            ),
        ),
        (
            "advisory-only failures are Fix CI, labelled advisory",
            pr("o/r", 1)
                .author(ME)
                .passing("ci / build")
                .failing_advisory("coverage"),
            expect(
                Some(FixCi),
                Me,
                "1 failing check, advisory only: coverage (advisory)",
                "PR author",
            ),
        ),
        (
            "required and advisory failures are each labelled",
            pr("o/r", 1)
                .failing_required("build")
                .failing_advisory("lint"),
            expect(
                Some(FixCi),
                SomeoneElse,
                "2 failing checks, 1 required and 1 advisory: build (required), lint (advisory)",
                "PR author",
            ),
        ),
        (
            "changes requested (GitHub's decision)",
            pr("o/r", 1).author(ME).changes_requested(),
            expect(Some(AddressReview), Me, "changes requested", "PR author"),
        ),
        (
            "changes requested in a latest review, no decision from GitHub",
            pr("o/r", 1)
                .author(ME)
                .reviewed("bob", ReviewVerdict::ChangesRequested),
            expect(
                Some(AddressReview),
                Me,
                "changes requested by bob",
                "PR author",
            ),
        ),
        (
            "unresolved review threads",
            pr("o/r", 1).author(ME).unresolved_threads(2),
            expect(
                Some(AddressReview),
                Me,
                "2 unresolved review threads",
                "PR author",
            ),
        ),
        (
            "review requested from me",
            pr("o/r", 1).author("alice").review_requested_from(ME),
            expect(
                Some(Review),
                Me,
                "review requested from you",
                "you are one of the requested reviewers",
            ),
        ),
        (
            "review requested from a team I belong to",
            pr("o/r", 1)
                .author("alice")
                .review_requested_from_team("octo", "core"),
            expect(
                Some(Review),
                Me,
                "review requested from your team octo/core",
                "your team octo/core is one of the requested reviewers",
            ),
        ),
        (
            "review requested from a team I'm not in, and another person",
            pr("o/r", 1)
                .author(ME)
                .review_requested_from_team("octo", "docs")
                .review_requested_from("bob"),
            expect(
                Some(Review),
                SomeoneElse,
                "review requested from team octo/docs, bob",
                "you are not one of the requested reviewers",
            ),
        ),
        (
            "approved, passing, mergeable",
            ready(1).author(ME),
            expect(
                Some(Merge),
                Me,
                "approved, checks passing and mergeable into main",
                "PR author",
            ),
        ),
        (
            "approved in a latest review, no decision from GitHub",
            pr("o/r", 1)
                .author(ME)
                .reviewed("bob", ReviewVerdict::Approved),
            expect(Some(Merge), Me, "approved", "PR author"),
        ),
        (
            "approved with no checks at all",
            pr("o/r", 1).author("alice").approved(),
            expect(Some(Merge), SomeoneElse, "approved", "PR author is alice"),
        ),
        // ── Adjacent precedence pairs ────────────────────────────────────
        (
            "draft beats conflicts",
            pr("o/r", 1).author(ME).draft().conflicting(),
            expect(Some(FinishDraft), Me, "draft", "PR author").also(&["conflicts with main"]),
        ),
        (
            "conflicts beat failing checks",
            pr("o/r", 1)
                .author(ME)
                .conflicting()
                .failing_required("build")
                .failing_advisory("lint"),
            expect(Some(ResolveConflicts), Me, "conflicts", "PR author")
                .also(&["2 failing checks, 1 required and 1 advisory"]),
        ),
        (
            "failing checks beat changes requested",
            pr("o/r", 1)
                .author(ME)
                .failing_required("build")
                .changes_requested(),
            expect(Some(FixCi), Me, "failing", "PR author").also(&["changes requested"]),
        ),
        (
            "changes requested beat a review request",
            pr("o/r", 1)
                .author(ME)
                .unresolved_threads(1)
                .review_requested_from("bob"),
            expect(
                Some(AddressReview),
                Me,
                "1 unresolved review thread",
                "PR author",
            )
            .also(&["review requested from bob"]),
        ),
        (
            "a review request beats Merge",
            ready(1).author("alice").review_requested_from(ME),
            expect(
                Some(Review),
                Me,
                "review requested from you",
                "requested reviewers",
            )
            .also(&["approved, checks passing and mergeable"]),
        ),
        (
            "everything at once: draft wins, the rest is listed",
            pr("o/r", 1)
                .author(ME)
                .draft()
                .conflicting()
                .failing_advisory("lint")
                .changes_requested()
                .review_requested_from(ME),
            expect(Some(FinishDraft), Me, "draft", "PR author").also(&[
                "conflicts with main",
                "1 failing check, advisory only: lint (advisory)",
                "changes requested",
                "review requested from you",
            ]),
        ),
        // ── What never yields Merge ──────────────────────────────────────
        (
            "unknown mergeability never yields Merge",
            ready(1).author(ME).mergeability_unknown(),
            expect(
                None,
                Nobody,
                "waiting for GitHub to compute mergeability",
                "no Next Step",
            )
            .unknown(&["mergeability not computed yet"]),
        ),
        (
            "a pending check yields neither Fix CI nor Merge",
            ready(1).author(ME).pending("ci / test"),
            expect(
                None,
                Nobody,
                "checks running: 1 pending (ci / test)",
                "no Next Step",
            ),
        ),
        (
            "a pending advisory check still holds Merge back",
            ready(1)
                .author(ME)
                .check("preview", false, CheckOutcome::Pending),
            expect(None, Nobody, "checks running", "no Next Step"),
        ),
        (
            "a queued PR never yields Merge",
            ready(1).author(ME).queued(),
            expect(None, Nobody, "already in the merge queue", "no Next Step"),
        ),
        // ── Merge without approval where no review is required ───────────
        (
            "no review decision, green and mergeable: Merge via no review required",
            pr("o/r", 1)
                .author(ME)
                .no_review_required()
                .passing("build"),
            expect(
                Some(Merge),
                Me,
                "no review required and no changes requested, checks passing and mergeable into main",
                "PR author",
            ),
        ),
        (
            "no review decision, changes requested in a latest review: Address review",
            pr("o/r", 1)
                .author(ME)
                .passing("build")
                .reviewed("bob", ReviewVerdict::ChangesRequested),
            expect(
                Some(AddressReview),
                Me,
                "changes requested by bob",
                "PR author",
            ),
        ),
        (
            "no review decision, an unresolved thread: Address review, not Merge",
            pr("o/r", 1)
                .author(ME)
                .no_review_required()
                .passing("build")
                .unresolved_threads(1),
            expect(
                Some(AddressReview),
                Me,
                "1 unresolved review thread",
                "PR author",
            ),
        ),
        (
            "no review decision, threads not all read: no Merge",
            pr("o/r", 1)
                .author(ME)
                .no_review_required()
                .passing("build")
                .threads_truncated(),
            expect(
                None,
                Nobody,
                "not every review thread was read",
                "no Next Step",
            )
            .unknown(&["not every review thread was read"]),
        ),
        (
            "no review decision, a pending check: no Merge",
            pr("o/r", 1)
                .author(ME)
                .no_review_required()
                .pending("build"),
            expect(None, Nobody, "checks running", "no Next Step"),
        ),
        (
            "review required by GitHub, green and mergeable: no Merge",
            pr("o/r", 1)
                .author(ME)
                .decision(ReviewDecision::ReviewRequired)
                .passing("build"),
            expect(
                None,
                Nobody,
                "not approved: a review is required",
                "no Next Step",
            ),
        ),
        (
            "a review required by GitHub but nobody requested",
            pr("o/r", 1)
                .author(ME)
                .decision(ReviewDecision::ReviewRequired),
            expect(None, Nobody, "a review is required", "no Next Step"),
        ),
        (
            "more checks than den read: no Merge",
            ready(1).author(ME).checks_truncated(),
            expect(None, Nobody, "not every check was read", "no Next Step")
                .unknown(&["more checks than den read"]),
        ),
        (
            "a check state den doesn't recognise: no Merge",
            ready(1)
                .author(ME)
                .check("odd", true, CheckOutcome::Unrecognised("WEIRD".to_owned())),
            expect(None, Nobody, "does not recognise", "no Next Step")
                .unknown(&["check odd reported \"WEIRD\""]),
        ),
        (
            "review requests not all read, none seen: no Merge",
            ready(1).author(ME).review_requests_truncated(),
            expect(
                None,
                Nobody,
                "not every review request was read",
                "no Next Step",
            )
            .unknown(&["some review requests could not be read"]),
        ),
        (
            "review decision unknown, green and mergeable: no Merge",
            pr("o/r", 1)
                .author(ME)
                .decision(ReviewDecision::Unknown)
                .passing("build"),
            expect(
                None,
                Nobody,
                "approval could not be established",
                "no Next Step",
            )
            .unknown(&["approval could not be established"]),
        ),
        (
            "review decision unknown, changes requested in a latest review: Address review",
            pr("o/r", 1)
                .author(ME)
                .passing("build")
                .reviewed("bob", ReviewVerdict::ChangesRequested)
                .decision(ReviewDecision::Unknown),
            expect(
                Some(AddressReview),
                Me,
                "changes requested by bob",
                "PR author",
            ),
        ),
        (
            "GitHub reports merging blocked (a required check never reported): no Merge",
            ready(1).author(ME).merge_state(MergeState::Blocked),
            expect(
                None,
                Nobody,
                "GitHub reports merging blocked by a rule den does not read",
                "no Next Step",
            ),
        ),
        (
            "behind a base that must be merged in first: no Merge",
            ready(1).author(ME).merge_state(MergeState::Behind),
            expect(None, Nobody, "behind main", "no Next Step"),
        ),
        (
            "GitHub's merge state not computed yet: no Merge",
            ready(1).author(ME).merge_state(MergeState::Unknown),
            expect(
                None,
                Nobody,
                "waiting for GitHub to compute whether it can merge",
                "no Next Step",
            )
            .unknown(&["merge state not computed yet"]),
        ),
        // ── Unknown facts ────────────────────────────────────────────────
        (
            "unknown mergeability with a failing check: Fix CI, conflicts not ruled out",
            pr("o/r", 1)
                .author(ME)
                .mergeability_unknown()
                .failing_required("build"),
            expect(
                Some(FixCi),
                Me,
                "earlier rules not ruled out: Resolve conflicts (mergeability not computed yet",
                "PR author",
            )
            .unknown(&["mergeability not computed yet"]),
        ),
        (
            "checks and reviews unread: a draft is still Finish draft",
            pr("o/r", 1).author(ME).draft().details_unread(),
            expect(Some(FinishDraft), Me, "draft", "PR author").unknown(&[
                "checks could not be read",
                "review threads could not be read",
                "review requests could not be read",
            ]),
        ),
        (
            "checks and reviews unread on an approved, mergeable PR: no Merge",
            pr("o/r", 1).author(ME).approved().details_unread(),
            expect(None, Nobody, "checks could not be read", "no Next Step")
                .unknown(&["checks could not be read"]),
        ),
        (
            "checks and reviews unread, no decision: approval unknown",
            pr("o/r", 1)
                .author(ME)
                .no_review_required()
                .details_unread(),
            expect(
                None,
                Nobody,
                "approval could not be established",
                "no Next Step",
            )
            .unknown(&["approval could not be established"]),
        ),
        (
            "a pending check alongside a winning rule is listed",
            pr("o/r", 1)
                .author(ME)
                .review_requested_from(ME)
                .pending("ci / test"),
            expect(Some(Review), Me, "review requested", "requested reviewers")
                .also(&["checks running: 1 pending (ci / test)"]),
        ),
        (
            "approved but conflicting: approval is listed",
            pr("o/r", 1).author(ME).approved().conflicting(),
            expect(Some(ResolveConflicts), Me, "conflicts", "PR author").also(&["approved"]),
        ),
    ]
}

#[test]
fn pr_rule_deck_gives_each_pr_its_next_step_waiting_on_and_explanation() {
    for (name, case, expect) in pr_rule_deck() {
        let observation = ObservationBuilder::new()
            .team("octo", "core")
            .owner("o")
            .pull_request(case)
            .build();
        let decisions = decisions(&observation);
        let decision = find(&decisions, &pr_id("o/r", 1));
        let explanation = &decision.explanation;
        assert_eq!(
            decision.next_step, expect.step,
            "{name}: step, {explanation:?}"
        );
        assert_eq!(decision.waiting_on, expect.waiting_on, "{name}: waiting on");
        assert_eq!(
            decision.unattributed, expect.unattributed,
            "{name}: unattributed"
        );
        assert!(
            decision.waiting_reason.contains(expect.waiting_reason),
            "{name}: Waiting On reason {:?}",
            decision.waiting_reason
        );
        let why = match expect.step {
            Some(_) => {
                assert_eq!(explanation.no_step_reason, None, "{name}");
                explanation.winning_rule.clone()
            }
            None => {
                assert_eq!(explanation.winning_rule, None, "{name}");
                explanation.no_step_reason.clone()
            }
        };
        assert!(
            why.as_deref().is_some_and(|w| w.contains(expect.why)),
            "{name}: expected {:?} in {explanation:?}",
            expect.why
        );
        for also in expect.also {
            assert!(
                explanation
                    .other_conditions
                    .iter()
                    .any(|c| c.contains(also)),
                "{name}: expected also {also:?} in {:?}",
                explanation.other_conditions
            );
        }
        for unknown in expect.unknown {
            assert!(
                explanation.unknowns.iter().any(|u| u.contains(unknown)),
                "{name}: expected unknown {unknown:?} in {:?}",
                explanation.unknowns
            );
        }
        if expect.unknown.is_empty() {
            assert_eq!(
                explanation.unknowns,
                Vec::<String>::new(),
                "{name}: no unknowns"
            );
        }
        // The winning rule is never repeated among the other conditions.
        if let Some(rule) = &explanation.winning_rule {
            assert!(
                !explanation.other_conditions.contains(rule),
                "{name}: {explanation:?}"
            );
        }
        assert_eq!(decision.observed_at, observation.observed_at, "{name}");
    }
}

#[test]
fn pending_or_unknown_never_yields_fix_ci_or_merge() {
    let held_back = [
        ready(1).pending("test"),
        ready(1).mergeability_unknown(),
        ready(1).queued(),
        ready(1).checks_truncated(),
        ready(1).details_unread(),
        pr("o/r", 1)
            .reviewed("bob", ReviewVerdict::Approved)
            .details_unread(),
    ];
    for case in held_back {
        let observation = ObservationBuilder::new()
            .owner("o")
            .pull_request(case)
            .build();
        let decision = decisions(&observation).remove(0);
        assert!(
            !matches!(decision.next_step, Some(NextStep::Merge | NextStep::FixCi)),
            "{decision:?}"
        );
    }
}

#[test]
fn every_pr_gets_exactly_one_decision_qualified_by_owner_and_repo() {
    let observation = ObservationBuilder::new()
        .owner("alpha")
        .owner("beta")
        .pull_request(pr("alpha/app", 1).draft())
        .pull_request(pr("alpha/lib", 1))
        .pull_request(pr("beta/app", 1).draft())
        .build();
    let decisions = decisions(&observation);
    assert_eq!(decisions.len(), 3);
    for (repo, step) in [
        ("alpha/app", Some(NextStep::FinishDraft)),
        ("alpha/lib", None),
        ("beta/app", Some(NextStep::FinishDraft)),
    ] {
        let decision = find(&decisions, &pr_id(repo, 1));
        assert_eq!(decision.next_step, step, "{repo}#1");
        assert_eq!(decision.item.id.repo(), &repo_id(repo));
    }
}

#[test]
fn partial_and_failed_coverage_are_carried_onto_the_affected_items_only() {
    let errors = SourceState::Partial(Incompleteness::ErrorsAlongsideData {
        errors: vec!["Something went wrong while executing your query.".to_owned()],
    });
    let failed = SourceState::Failed {
        error: "timed out".to_owned(),
    };
    let observation = ObservationBuilder::new()
        .owner("alpha")
        .owner("beta")
        .owner_coverage("alpha", SourceKind::PullRequests, errors.clone())
        .repo_coverage("beta/big", SourceKind::PullRequests, failed.clone())
        .pull_request(pr("alpha/app", 1))
        .pull_request(pr("beta/big", 2))
        .pull_request(pr("beta/small", 3))
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
    assert_eq!(incomplete(&pr_id("alpha/app", 1)), vec![errors]);
    assert_eq!(incomplete(&pr_id("beta/big", 2)), vec![failed]);
    assert_eq!(
        incomplete(&pr_id("beta/small", 3)),
        Vec::<SourceState>::new()
    );

    // Applicable coverage names the sources, and never other kinds' sources.
    let sources: Vec<Source> = find(&decisions, &pr_id("beta/small", 3))
        .coverage
        .iter()
        .map(|c| c.source.clone())
        .collect();
    assert!(sources.contains(&Source::Viewer));
    assert!(sources.contains(&Source::Owner {
        owner: "beta".to_owned(),
        kind: SourceKind::PullRequests
    }));
    for source in &sources {
        match source {
            Source::Viewer => {}
            Source::Owner { owner, kind } => {
                assert_eq!((owner.as_str(), *kind), ("beta", SourceKind::PullRequests));
            }
            Source::Repo { repo, kind } => {
                assert_eq!(
                    (repo, *kind),
                    (&repo_id("beta/small"), SourceKind::PullRequests)
                );
            }
        }
    }
}

#[test]
fn deciding_is_deterministic_and_reads_no_clock() {
    let observation = ObservationBuilder::new()
        .owner("o")
        .pull_request(pr("o/r", 1).draft())
        .pull_request(pr("o/r", 2))
        .build();
    let first = decide(&observation, &observation.viewer, now());
    let second = decide(&observation, &observation.viewer, now());
    assert_eq!(first, second);
}

#[test]
fn the_domain_names_all_ten_next_steps_with_their_glossary_labels() {
    let labels: Vec<&str> = NextStep::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(
        labels,
        [
            "Finish draft",
            "Resolve conflicts",
            "Fix CI",
            "Address review",
            "Review",
            "Merge",
            "Triage",
            "Implement",
            "Open PR",
            "Prune or revive",
        ]
    );
}

/// Who a requested review belongs to, when den may not have read every request
/// or every team of the viewer's: a match to the viewer still counts, but "not
/// you" needs the whole picture; without it the review is unattributed.
#[test]
fn a_review_den_cannot_rule_out_as_mine_is_unattributed_not_someone_else_s() {
    use NextStep::Review;
    use WaitingOn::{Me, Nobody, SomeoneElse};
    let teams_unknown = SourceState::Partial(Incompleteness::ErrorsAlongsideData {
        errors: vec!["team memberships unknown: timed out".to_owned()],
    });
    let cases = [
        (
            "a team requested, my teams not all read",
            ObservationBuilder::new()
                .team("octo", "core")
                .viewer_coverage(teams_unknown.clone())
                .pull_request(
                    pr("o/r", 1)
                        .author("alice")
                        .review_requested_from_team("octo", "docs"),
                ),
            Nobody,
            true,
            "your team memberships",
        ),
        (
            "requests not all read, the ones read are someone else",
            ObservationBuilder::new().team("octo", "core").pull_request(
                pr("o/r", 1)
                    .author("alice")
                    .review_requested_from("bob")
                    .review_requests_truncated(),
            ),
            Nobody,
            true,
            "not every review request was read",
        ),
        (
            "my team requested, my teams not all read: still mine",
            ObservationBuilder::new()
                .team("octo", "core")
                .viewer_coverage(teams_unknown.clone())
                .pull_request(
                    pr("o/r", 1)
                        .author("alice")
                        .review_requested_from_team("octo", "core"),
                ),
            Me,
            false,
            "your team octo/core",
        ),
        (
            "me requested, requests not all read: still mine",
            ObservationBuilder::new().team("octo", "core").pull_request(
                pr("o/r", 1)
                    .author("alice")
                    .review_requested_from(ME)
                    .review_requests_truncated(),
            ),
            Me,
            false,
            "you are one of",
        ),
        (
            "only people requested, my teams not all read: teams don't matter",
            ObservationBuilder::new()
                .team("octo", "core")
                .viewer_coverage(teams_unknown)
                .pull_request(pr("o/r", 1).author("alice").review_requested_from("bob")),
            SomeoneElse,
            false,
            "you are not one of",
        ),
    ];
    for (name, builder, waiting_on, unattributed, reason) in cases {
        let observation = builder.owner("o").build();
        let decisions = decisions(&observation);
        let decision = find(&decisions, &pr_id("o/r", 1));
        assert_eq!(decision.next_step, Some(Review), "{name}");
        assert_eq!(decision.waiting_on, waiting_on, "{name}");
        assert_eq!(decision.unattributed, unattributed, "{name}");
        assert!(
            decision.waiting_reason.contains(reason),
            "{name}: {:?}",
            decision.waiting_reason
        );
    }
}
