//! Seam 1, milestone-2 groundwork (#31): the decision engine takes each Owner's
//! Linear Workflow policy, and an Owner without one (an empty policy) is
//! decided exactly as milestone 1 decided it.
//!
//! The deck below covers every milestone-1 Next Step, Waiting On me, someone
//! else and nobody, two Owners with the same Repo name and PR number, and
//! incomplete coverage. `tests/fixtures/milestone-1/decisions.json` holds what
//! milestone 1's own `decide` (den at `ab5a940`) returned for it.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        Incompleteness, Observation, SourceKind, SourceState,
        facts::{LinkKind, LinkedPrState, ReviewVerdict},
    },
};
use std::path::PathBuf;
use support::build::{ME, ObservationBuilder, branch, issue, now, pr, run};

const OTHER: &str = "someone";

fn milestone_1_decisions_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/milestone-1/decisions.json")
}

/// Two Owners, each with an `app` Repo and a PR #1, every milestone-1 Next
/// Step, and one Owner whose Issues were read only in part.
fn deck() -> Observation {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/app")
        .repo("acme/api")
        .fully_read_owner("octo")
        .repo("octo/app")
        .owner_coverage(
            "octo",
            SourceKind::Issues,
            SourceState::Partial(Incompleteness::PaginationStopped {
                pages_read: 2,
                reason: "a page timed out".to_owned(),
            }),
        )
        // Pull Requests: one per milestone-1 PR Next Step, and none at all.
        .pull_request(pr("acme/app", 1).author(ME).draft().conflicting())
        .pull_request(pr("acme/app", 2).author(ME).conflicting())
        .pull_request(pr("acme/app", 3).author(ME).failing_required("ci"))
        .pull_request(pr("acme/app", 4).author(ME).changes_requested().unresolved_threads(2))
        .pull_request(
            pr("acme/app", 5)
                .author(OTHER)
                .review_requested_from(ME)
                .passing("ci"),
        )
        .pull_request(pr("acme/app", 6).author(ME).approved().passing("ci"))
        .pull_request(pr("acme/app", 7).author(ME).approved().mergeability_unknown())
        .pull_request(pr("acme/app", 8).author(ME).approved().passing("ci").queued())
        .pull_request(
            pr("acme/api", 9)
                .author(OTHER)
                .reviewed(ME, ReviewVerdict::ChangesRequested)
                .changes_requested(),
        )
        .pull_request(pr("acme/api", 10).no_author().details_unread())
        .pull_request(pr("octo/app", 1).author(OTHER).approved().passing("ci"))
        // Issues: Triage, Implement, carried by a linked PR, assigned elsewhere.
        .issue(issue("acme/app", 20))
        .issue(issue("acme/app", 21).label("bug"))
        .issue(issue("acme/app", 22).label("bug").linked(
            LinkKind::ClosingReference,
            "acme/app",
            2,
            LinkedPrState::Open,
        ))
        .issue(issue("acme/api", 23).assignee(OTHER))
        .issue(issue("octo/app", 1).assignee(ME).labels_unread())
        // Branches: Open PR and Prune or revive.
        .branch(branch("acme/app", "feature-x").author(ME).ahead(3))
        .branch(branch("acme/app", "old-idea").author(ME).ahead(1).age_days(45))
        .branch(branch("octo/app", "feature-x").unmatched_author().ahead(2))
        // Runs: a failing and a passing workflow on the default branch.
        .run(run("acme/app", "ci.yml", 100).failed().by(ME))
        .run(run("acme/api", "ci.yml", 101).by(OTHER))
        .run(run("octo/app", "release.yml", 102).failed().no_actor())
        .build()
}

fn milestone_1(observation: &Observation) -> Vec<Decision> {
    decide(observation, &observation.viewer, now())
}

/// Not a test of den: run once, by hand, on milestone 1's code to write the
/// golden file. Kept so the deck and the file can be seen to match.
#[test]
#[ignore = "writes the golden file; run only on milestone 1's code"]
fn write_milestone_1_decisions() {
    let decisions = milestone_1(&deck());
    let body = serde_json::to_vec_pretty(&decisions).unwrap();
    std::fs::write(milestone_1_decisions_file(), body).unwrap();
}
