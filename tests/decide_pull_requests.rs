//! Seam 1: the decision engine on Pull Requests. Observation in, Decisions out.
//!
//! This ticket implements only the Finish draft rule; the PR rules ticket (#4)
//! extends the case deck with every other rule and precedence pair.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        Incompleteness, NextStep, Observation, Source, SourceKind, SourceState, WaitingOn,
        WorkItemId,
    },
};
use support::build::{ME, ObservationBuilder, now, pr, repo_id};

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
}

#[test]
fn finish_draft_rule_deck() {
    let cases = [
        (
            "my draft",
            pr("o/r", 1).draft().author(ME),
            Expect {
                step: Some(NextStep::FinishDraft),
                waiting_on: WaitingOn::Me,
                unattributed: false,
                why: "draft",
            },
        ),
        (
            "my draft, login in another case",
            pr("o/r", 1).draft().author("ME"),
            Expect {
                step: Some(NextStep::FinishDraft),
                waiting_on: WaitingOn::Me,
                unattributed: false,
                why: "draft",
            },
        ),
        (
            "someone else's draft",
            pr("o/r", 1).draft().author("alice"),
            Expect {
                step: Some(NextStep::FinishDraft),
                waiting_on: WaitingOn::SomeoneElse,
                unattributed: false,
                why: "draft",
            },
        ),
        (
            "a draft whose author is gone",
            pr("o/r", 1).draft().no_author(),
            Expect {
                step: Some(NextStep::FinishDraft),
                waiting_on: WaitingOn::Nobody,
                unattributed: true,
                why: "draft",
            },
        ),
        (
            "a ready PR of mine",
            pr("o/r", 1).author(ME),
            Expect {
                step: None,
                waiting_on: WaitingOn::Nobody,
                unattributed: false,
                why: "not a draft",
            },
        ),
        (
            "someone else's ready PR",
            pr("o/r", 1).author("alice"),
            Expect {
                step: None,
                waiting_on: WaitingOn::Nobody,
                unattributed: false,
                why: "not a draft",
            },
        ),
    ];
    for (name, case, expect) in cases {
        let observation = ObservationBuilder::new()
            .owner("o")
            .pull_request(case)
            .build();
        let decisions = decisions(&observation);
        let decision = find(&decisions, &pr_id("o/r", 1));
        assert_eq!(decision.next_step, expect.step, "{name}: step");
        assert_eq!(decision.waiting_on, expect.waiting_on, "{name}: waiting on");
        assert_eq!(
            decision.unattributed, expect.unattributed,
            "{name}: unattributed"
        );
        assert!(
            !decision.waiting_reason.is_empty(),
            "{name}: a Waiting On reason"
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
            !decision.explanation.unknowns.is_empty(),
            "{name}: the unevaluated PR rules are listed as unknown"
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
