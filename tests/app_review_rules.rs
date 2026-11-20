//! Seam 2, the Linear Workflow's review rules (#36): the config's review
//! settings reach the decision engine; a held PR reads "held for release" in
//! the list and the detail; and the detail shows the suggested reviewers.
//! Launching Request review is `app_request_review.rs` (#39).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Key, view_model::ViewModel},
    config::{self, Config},
    domain::{RepoId, TeamRef, facts::ReviewVerdict},
};
use support::{
    build::{ME, ObservationBuilder, pr},
    harness::{Harness, config_with_linear},
};

/// `acme` with a Linear Workflow: Held Repo `app`, reviewers alice and the
/// team acme/backend; `octo` without one.
fn config() -> Config {
    let mut config = config_with_linear(&["acme", "octo"], &[("acme", "acme")]);
    let workflow = &mut config.linear[0];
    workflow.held_repos = vec![RepoId::new("acme", "app")];
    workflow.reviewers = vec![
        config::Reviewer::User("alice".to_owned()),
        config::Reviewer::Team(TeamRef {
            org: "acme".to_owned(),
            slug: "backend".to_owned(),
        }),
    ];
    config
}

/// Three ready PRs by me: one in the Held Repo, one approved only at an older
/// head (Request review under the workflow), and the same under `octo`.
fn started() -> Harness {
    let mut harness = Harness::new(Ok(config()));
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/app")
        .repo("acme/api")
        .fully_read_owner("octo")
        .repo("octo/api")
        .pull_request(
            pr("acme/app", 1)
                .author(ME)
                .passing("ci")
                .approved()
                .review_at_head("alice", ReviewVerdict::Approved),
        )
        .pull_request(
            pr("acme/api", 2)
                .author(ME)
                .passing("ci")
                .approved()
                .review_on("bob", ReviewVerdict::Approved, "0ld"),
        )
        .pull_request(
            pr("octo/api", 2)
                .author(ME)
                .passing("ci")
                .approved()
                .review_on("bob", ReviewVerdict::Approved, "0ld"),
        )
        .build()));
    harness.start();
    harness
}

fn select(harness: &mut Harness, repo: &str, reference: &str) -> ViewModel {
    let index = harness
        .view
        .list
        .rows
        .iter()
        .position(|r| r.repo == repo && r.reference == reference)
        .unwrap_or_else(|| panic!("no row {repo} {reference}: {:#?}", harness.view.list.rows));
    let current = harness.view.list.selected.unwrap_or(0);
    let key = if index >= current { Key::Down } else { Key::Up };
    for _ in 0..index.abs_diff(current) {
        harness.key(key);
    }
    harness.view.clone()
}

#[test]
fn a_held_pr_reads_held_for_release_in_the_list_and_the_detail() {
    let mut harness = started();
    let view = select(&mut harness, "acme/app", "#1");
    let row = &view.list.rows[view.list.selected.unwrap()];
    assert_eq!(row.next_step, None);
    assert_eq!(row.step_label, "held for release");

    let detail = view.detail.content.expect("selected");
    assert_eq!(detail.next_step, "none (held for release)");
    assert!(detail.why.contains("acme/app is a Held Repo"), "{detail:?}");
    let facts = detail.facts.expect("a workflow section");
    assert_eq!(facts.label, "LINEAR WORKFLOW");
    assert!(
        facts.lines.contains(&"held for release".to_owned()),
        "{facts:?}"
    );
}

#[test]
fn the_detail_shows_the_suggested_reviewers_of_a_workflow_pr_only() {
    let mut harness = started();
    let view = select(&mut harness, "acme/api", "#2");
    let detail = view.detail.content.expect("selected");
    assert_eq!(detail.next_step, "Request review");
    let facts = detail.facts.expect("a workflow section");
    assert_eq!(
        facts.lines,
        ["suggested reviewers: alice, team acme/backend"],
        "bob approved an older head: not awaiting re-review, so not suggested"
    );

    // The same facts under octo, which has no Linear Workflow: milestone 1.
    let view = select(&mut harness, "octo/api", "#2");
    let detail = view.detail.content.expect("selected");
    assert_eq!(detail.next_step, "Merge");
    assert_eq!(detail.facts, None);
}

#[test]
fn the_detail_says_when_the_suggested_reviewers_are_not_filtered_in_full() {
    let mut harness = Harness::new(Ok(config()));
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .pull_request(
            pr("acme/api", 3)
                .author(ME)
                .passing("ci")
                .approved()
                .review_requests_truncated(),
        )
        .build()));
    harness.start();
    let view = select(&mut harness, "acme/api", "#3");
    let facts = view
        .detail
        .content
        .expect("selected")
        .facts
        .expect("a workflow section");
    assert_eq!(
        facts.lines,
        [
            "suggested reviewers: alice, team acme/backend",
            "(may include someone already asked or approved: not every review request was read)",
        ]
    );
}
