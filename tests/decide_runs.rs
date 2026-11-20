//! Seam 1: the decision engine on Runs. Observation in, Decisions out.
//!
//! Rule (spec, "Next Step rules"): per workflow, only the latest attempt of the
//! newest Run on the default branch counts; if it failed → Fix CI, Waiting On
//! the Run's triggering actor. Anything else is not a Work Item.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        Incompleteness, NextStep, Observation, Source, SourceKind, SourceState, WaitingOn,
        WorkItemId, WorkflowId, facts::RunConclusion,
    },
};
use support::build::{ME, ObservationBuilder, RunBuilder, now, repo_id, run, ts};

fn decisions(observation: &Observation) -> Vec<Decision> {
    decide(observation, &observation.viewer, now())
}

fn run_id(repo: &str, workflow: &str) -> WorkItemId {
    WorkItemId::Run {
        repo: repo_id(repo),
        workflow: WorkflowId(format!(".github/workflows/{workflow}")),
    }
}

/// What one Run Decision must say.
struct Expect {
    item: WorkItemId,
    waiting_on: WaitingOn,
    unattributed: bool,
    /// Substrings of the winning rule.
    rule: &'static [&'static str],
    /// Substrings that must each appear in some other condition.
    also: &'static [&'static str],
}

fn fix_ci(repo: &str, workflow: &str, waiting_on: WaitingOn) -> Expect {
    Expect {
        item: run_id(repo, workflow),
        waiting_on,
        unattributed: false,
        rule: &["latest Run", "default branch", "failed"],
        also: &[],
    }
}

fn check(case: &str, runs: Vec<RunBuilder>, expected: &[Expect]) {
    let observation = runs
        .into_iter()
        .fold(ObservationBuilder::new().fully_read_owner("o"), |b, r| {
            b.run(r)
        })
        .build();
    let decided = decisions(&observation);
    let ids: Vec<&WorkItemId> = decided.iter().map(|d| &d.item.id).collect();
    let expected_ids: Vec<&WorkItemId> = expected.iter().map(|e| &e.item).collect();
    assert_eq!(ids, expected_ids, "{case}: which Work Items");
    for (decision, expect) in decided.iter().zip(expected) {
        let who = &decision.item.id;
        assert_eq!(decision.next_step, Some(NextStep::FixCi), "{case}: {who}");
        assert_eq!(decision.waiting_on, expect.waiting_on, "{case}: {who}");
        assert_eq!(decision.unattributed, expect.unattributed, "{case}: {who}");
        let rule = decision
            .explanation
            .winning_rule
            .as_deref()
            .unwrap_or_else(|| panic!("{case}: {who} has no winning rule"));
        for part in expect.rule {
            assert!(
                rule.contains(part),
                "{case}: {who}: rule {rule:?} lacks {part:?}"
            );
        }
        for part in expect.also {
            assert!(
                decision
                    .explanation
                    .other_conditions
                    .iter()
                    .any(|c| c.contains(part)),
                "{case}: {who}: no other condition with {part:?} in {:?}",
                decision.explanation.other_conditions
            );
        }
    }
}

#[test]
fn each_workflow_is_judged_on_its_own_latest_run() {
    check(
        "workflow A passes and B fails: one Fix CI, for B",
        vec![run("o/r", "a.yml", 20), run("o/r", "b.yml", 10).failed()],
        &[fix_ci("o/r", "b.yml", WaitingOn::SomeoneElse)],
    );
    check(
        "a failed attempt replaced by a successful retry: no Work Item",
        vec![
            run("o/r", "ci.yml", 10).failed(),
            run("o/r", "ci.yml", 10).attempt(2),
        ],
        &[],
    );
    check(
        "the retry listed before the attempt it replaced: still no Work Item",
        vec![
            run("o/r", "ci.yml", 10).attempt(2),
            run("o/r", "ci.yml", 10).failed(),
        ],
        &[],
    );
    check(
        "a newer success after an older failure: no Work Item",
        vec![run("o/r", "ci.yml", 10).failed(), run("o/r", "ci.yml", 11)],
        &[],
    );
    check(
        "a newer failure after an older success: Fix CI",
        vec![run("o/r", "ci.yml", 11).failed(), run("o/r", "ci.yml", 10)],
        &[fix_ci("o/r", "ci.yml", WaitingOn::SomeoneElse)],
    );
    check(
        "a successful retry of an older Run does not clear a newer failure",
        vec![
            run("o/r", "ci.yml", 10).attempt(2),
            run("o/r", "ci.yml", 11).failed(),
        ],
        &[fix_ci("o/r", "ci.yml", WaitingOn::SomeoneElse)],
    );
    check(
        "a failed retry of a Run whose first attempt passed: Fix CI",
        vec![
            run("o/r", "ci.yml", 10),
            run("o/r", "ci.yml", 10).attempt(2).failed(),
        ],
        &[Expect {
            also: &["attempt 2"],
            ..fix_ci("o/r", "ci.yml", WaitingOn::SomeoneElse)
        }],
    );
    check(
        "two failing workflows: two Work Items",
        vec![
            run("o/r", "a.yml", 10).failed(),
            run("o/r", "b.yml", 11).failed(),
        ],
        &[
            fix_ci("o/r", "a.yml", WaitingOn::SomeoneElse),
            fix_ci("o/r", "b.yml", WaitingOn::SomeoneElse),
        ],
    );
    check(
        "the same workflow failing in two Repos: two Work Items, one per Repo",
        vec![
            run("o/one", "ci.yml", 10).failed(),
            run("o/two", "ci.yml", 10).failed(),
        ],
        &[
            fix_ci("o/one", "ci.yml", WaitingOn::SomeoneElse),
            fix_ci("o/two", "ci.yml", WaitingOn::SomeoneElse),
        ],
    );
}

#[test]
fn runs_off_the_default_branch_never_make_work_items() {
    check(
        "a failed Pull Request run, and a failed push to a feature branch",
        vec![
            run("o/r", "ci.yml", 10)
                .failed()
                .off_default_branch("feature", "pull_request"),
            run("o/r", "deploy.yml", 11)
                .failed()
                .off_default_branch("feature", "push"),
        ],
        &[],
    );
    check(
        "a newer failed run off the default branch does not hide a passing default-branch Run",
        vec![
            run("o/r", "ci.yml", 10),
            run("o/r", "ci.yml", 11)
                .failed()
                .off_default_branch("feature", "push"),
        ],
        &[],
    );
    check(
        "a newer passing run off the default branch does not clear a default-branch failure",
        vec![
            run("o/r", "ci.yml", 10).failed(),
            run("o/r", "ci.yml", 11).off_default_branch("feature", "pull_request"),
        ],
        &[fix_ci("o/r", "ci.yml", WaitingOn::SomeoneElse)],
    );
}

#[test]
fn only_a_completed_failure_needs_fix_ci() {
    for (conclusion, needs_fix) in [
        (RunConclusion::Failure, true),
        (RunConclusion::TimedOut, true),
        (RunConclusion::StartupFailure, true),
        (RunConclusion::Success, false),
        (RunConclusion::Cancelled, false),
        (RunConclusion::Skipped, false),
        (RunConclusion::ActionRequired, false),
        (RunConclusion::Neutral, false),
        (RunConclusion::Stale, false),
        (RunConclusion::Unknown("mystery".to_owned()), false),
    ] {
        let case = format!("latest Run concluded {conclusion:?}");
        let expected: &[Expect] = if needs_fix {
            &[Expect {
                rule: &["latest Run", "default branch"],
                ..fix_ci("o/r", "ci.yml", WaitingOn::SomeoneElse)
            }]
        } else {
            &[]
        };
        check(
            &case,
            vec![run("o/r", "ci.yml", 10).concluded(conclusion)],
            expected,
        );
    }
    check(
        "a Run still in progress after an older failure: not failed yet, no Work Item",
        vec![
            run("o/r", "ci.yml", 10).failed(),
            run("o/r", "ci.yml", 11).in_progress(),
        ],
        &[],
    );
    check(
        "a retry still in progress: no Work Item",
        vec![
            run("o/r", "ci.yml", 10).failed(),
            run("o/r", "ci.yml", 10).attempt(2).in_progress(),
        ],
        &[],
    );
}

#[test]
fn fix_ci_is_waiting_on_the_triggering_actor() {
    check(
        "I pushed the commit that broke it",
        vec![run("o/r", "ci.yml", 10).failed().by(ME)],
        &[fix_ci("o/r", "ci.yml", WaitingOn::Me)],
    );
    check(
        "logins compare case-insensitively",
        vec![run("o/r", "ci.yml", 10).failed().by("Me")],
        &[fix_ci("o/r", "ci.yml", WaitingOn::Me)],
    );
    check(
        "someone else's scheduled Run",
        vec![
            run("o/r", "ci.yml", 10)
                .failed()
                .by("alice")
                .event("schedule"),
        ],
        &[Expect {
            also: &["schedule"],
            ..fix_ci("o/r", "ci.yml", WaitingOn::SomeoneElse)
        }],
    );
    check(
        "I re-ran someone else's Run and it failed again: the retry is mine",
        vec![
            run("o/r", "ci.yml", 10)
                .attempt(2)
                .failed()
                .by("alice")
                .retried_by(ME),
        ],
        &[Expect {
            also: &["first started by alice"],
            ..fix_ci("o/r", "ci.yml", WaitingOn::Me)
        }],
    );
    check(
        "no triggering actor on a first attempt: whoever started the Run triggered it",
        vec![run("o/r", "ci.yml", 10).failed().started_by_only("alice")],
        &[fix_ci("o/r", "ci.yml", WaitingOn::SomeoneElse)],
    );
    check(
        "no triggering actor on a retry: the first attempt's actor may not have re-run it",
        vec![
            run("o/r", "ci.yml", 10)
                .attempt(2)
                .failed()
                .started_by_only(ME),
        ],
        &[Expect {
            unattributed: true,
            ..fix_ci("o/r", "ci.yml", WaitingOn::Nobody)
        }],
    );
    check(
        "GitHub reported no actor: Waiting On nobody, unattributed, kept visible",
        vec![run("o/r", "ci.yml", 10).failed().no_actor()],
        &[Expect {
            unattributed: true,
            ..fix_ci("o/r", "ci.yml", WaitingOn::Nobody)
        }],
    );
}

#[test]
fn an_unattributed_run_says_what_is_unknown_and_why_it_waits() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .run(run("o/r", "ci.yml", 10).failed().no_actor())
        .build();
    let decided = decisions(&observation);
    let [decision] = decided.as_slice() else {
        panic!("one decision, got {decided:?}");
    };
    assert!(
        decision
            .explanation
            .unknowns
            .iter()
            .any(|u| u.contains("who triggered")),
        "{:?}",
        decision.explanation.unknowns
    );
    assert!(
        decision.waiting_reason.contains("triggering actor"),
        "{}",
        decision.waiting_reason
    );
    assert_eq!(decision.item.title, "ci");
    assert_eq!(decision.item.url, "https://github.com/o/r/actions/runs/10");
}

#[test]
fn an_unattributed_retry_names_only_who_started_the_first_attempt() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .run(
            run("o/r", "ci.yml", 10)
                .attempt(3)
                .failed()
                .started_by_only("alice"),
        )
        .build();
    let decided = decisions(&observation);
    let [decision] = decided.as_slice() else {
        panic!("one decision, got {decided:?}");
    };
    assert!(
        decision
            .explanation
            .unknowns
            .iter()
            .any(|u| u.contains("who triggered attempt 3") && u.contains("alice")),
        "{:?}",
        decision.explanation.unknowns
    );
}

#[test]
fn a_run_item_is_dated_by_when_its_failing_attempt_started() {
    let observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .run(
            run("o/r", "ci.yml", 10)
                .failed()
                .started("2025-12-24T14:06:06Z"),
        )
        .run(
            run("o/r", "lint.yml", 11)
                .failed()
                .only_updated("2026-11-15T02:10:26Z"),
        )
        .build();
    let decided = decisions(&observation);
    let dated = |workflow: &str| {
        decided
            .iter()
            .find(|d| d.item.id == run_id("o/r", workflow))
            .unwrap_or_else(|| panic!("no decision for {workflow}"))
            .item
            .updated_at
    };
    assert_eq!(dated("ci.yml"), ts("2025-12-24T14:06:06Z"));
    assert_eq!(dated("lint.yml"), ts("2026-11-15T02:10:26Z"));
}

#[test]
fn partial_and_failed_runs_coverage_is_carried_onto_run_items() {
    let partial = SourceState::Partial(Incompleteness::PaginationStopped {
        pages_read: 1,
        reason: "200 workflows; den reads at most 100".to_owned(),
    });
    let failed = SourceState::Failed {
        error: "GitHub answered 404".to_owned(),
    };
    let observation = ObservationBuilder::new()
        .fully_read_owner("o")
        .fully_read_owner("p")
        .repo_coverage("o/busy", SourceKind::Runs, partial.clone())
        .owner_coverage("p", SourceKind::Runs, failed.clone())
        .run(run("o/busy", "ci.yml", 10).failed())
        .run(run("o/calm", "ci.yml", 11).failed())
        .run(run("p/r", "ci.yml", 12).failed())
        .build();
    let decided = decisions(&observation);
    let coverage_of = |repo: &str| -> Vec<(Source, SourceState)> {
        decided
            .iter()
            .find(|d| d.item.id == run_id(repo, "ci.yml"))
            .unwrap_or_else(|| panic!("no decision for {repo}"))
            .coverage
            .iter()
            .filter(|e| e.source != Source::Viewer)
            .map(|e| (e.source.clone(), e.state.clone()))
            .collect()
    };
    let owner = |owner: &str| Source::Owner {
        owner: owner.to_owned(),
        kind: SourceKind::Runs,
    };
    assert_eq!(
        coverage_of("o/busy"),
        [
            (owner("o"), SourceState::Complete),
            (
                Source::Repo {
                    repo: repo_id("o/busy"),
                    kind: SourceKind::Runs
                },
                partial
            ),
        ]
    );
    assert_eq!(coverage_of("o/calm"), [(owner("o"), SourceState::Complete)]);
    assert_eq!(coverage_of("p/r"), [(owner("p"), failed)]);
}
