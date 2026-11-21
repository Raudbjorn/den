//! Seam 1, Add screenshots (#79; spec 0004, stories 55–63). A Repo with
//! `screenshots = true` needs the viewer's own Pull Requests to say how they
//! look: a `## Screenshots` section with an image, or a "No visual change"
//! line. Until one does, its Next Step is **Add screenshots**, after Address
//! review and before Review and Merge, and Merge is withheld. Other people's
//! PRs, and Repos without the setting, are never held for screenshots.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, decide},
    domain::{
        NextStep, Observation, Policies, RepoId, Skips, Viewers, WaitingOn, facts::Screenshots,
    },
};
use support::build::{ME, ObservationBuilder, PrBuilder, now, pr};

// Decoding the PR body (only the state is kept, never the body).

#[test]
fn a_screenshots_section_with_an_image_has_screenshots() {
    for body in [
        "Fixes the header.\n\n## Screenshots\n\n| Before | After |\n|---|---|\n| ![before](https://x/b.png) | ![after](https://x/a.png) |\n",
        "## Screenshots\n<img width=\"300\" src=\"https://x/a.png\">\n",
        "## screenshots  \n\n![](https://x/a.png)",
    ] {
        assert_eq!(
            Screenshots::from_body(body),
            Screenshots::Present,
            "{body:?}"
        );
    }
}

#[test]
fn a_no_visual_change_line_says_so() {
    for body in [
        "Refactor only.\n\nNo visual change\n",
        "## Screenshots\n\nNo visual change.\n",
        "  no visual change  ",
    ] {
        assert_eq!(
            Screenshots::from_body(body),
            Screenshots::NoVisualChange,
            "{body:?}"
        );
    }
}

#[test]
fn anything_else_is_missing() {
    for body in [
        "",
        "Fixes the header.",
        // A section without an image.
        "## Screenshots\n\nTODO\n",
        // An image outside the section.
        "![logo](https://x/l.png)\n\n## Screenshots\n\nsoon\n\n## Notes\n",
        // An image under the next section, not this one.
        "## Screenshots\n\nTODO\n\n## Notes\n\n![x](https://x/x.png)\n",
        // A sentence that mentions it is not the line.
        "There is no visual change here, I think.",
    ] {
        assert_eq!(
            Screenshots::from_body(body),
            Screenshots::Missing,
            "{body:?}"
        );
    }
}

/// HTML comments are ignored: a PR template's commented-out example or hint
/// never counts.
#[test]
fn html_comments_never_count() {
    for body in [
        "## Screenshots\n<!-- ![example](https://x/e.png) -->\n",
        "<!-- No visual change -->",
        "## Screenshots\n<!-- unterminated ![e](https://x/e.png)\nNo visual change",
    ] {
        assert_eq!(
            Screenshots::from_body(body),
            Screenshots::Missing,
            "{body:?}"
        );
    }
}

/// Review on #94: reference-style images (`![alt][label]`, `![label][]`,
/// `![label]`) display as images when the body defines the label, anywhere,
/// so they count. Labels match as CommonMark matches them, ignoring case
/// and runs of whitespace. An undefined label displays as text, so it
/// doesn't count, and neither does a definition inside an HTML comment or a
/// footnote definition.
#[test]
fn reference_style_images_count_when_their_label_is_defined() {
    for body in [
        "## Screenshots\n\n![after][shot]\n\n[shot]: https://x/a.png\n",
        "## Screenshots\n\n| Before | After |\n|---|---|\n| ![b][Before  Shot] | ![a][after] |\n\n[before shot]: https://x/b.png\n[AFTER]: <https://x/a.png> \"after\"\n",
        "## Screenshots\n![shot][]\n\n[shot]: https://x/a.png",
        "[shot]: https://x/a.png\n\n## Screenshots\n![shot]\n",
    ] {
        assert_eq!(
            Screenshots::from_body(body),
            Screenshots::Present,
            "{body:?}"
        );
    }
    for body in [
        "## Screenshots\n\n![after][shot]\n",
        "## Screenshots\n\n![shot]\n",
        "## Screenshots\n\n![after][shot]\n<!-- [shot]: https://x/a.png -->\n",
        "## Screenshots\n\nsoon\n\n## Notes\n\n![x][shot]\n\n[shot]: https://x/a.png\n",
        // A full reference names its label; its alt text being defined
        // doesn't make it an image.
        "## Screenshots\n\n![shot][other]\n\n[shot]: https://x/a.png\n",
        // A footnote definition defines no link.
        "## Screenshots\n\n![^1]\n\n[^1]: a note\n",
    ] {
        assert_eq!(
            Screenshots::from_body(body),
            Screenshots::Missing,
            "{body:?}"
        );
    }
}

// The decision.

const APP: &str = "octo/app";

fn screenshots_in_app() -> Policies {
    Policies::default().with_screenshot_repos(vec![RepoId::new("octo", "app")])
}

/// A PR of mine that would otherwise Merge: approved, checks passing.
fn mine_ready(number: u64) -> PrBuilder {
    pr(APP, number).author(ME).approved().passing("ci")
}

fn decide_one(pr: PrBuilder, policies: &Policies) -> Decision {
    let observation: Observation = ObservationBuilder::new()
        .fully_read_owner("octo")
        .repo(APP)
        .pull_request(pr)
        .build();
    let mut decisions = decide(&observation, &Viewers::of(&observation), policies, now());
    assert_eq!(decisions.len(), 1, "{decisions:#?}");
    decisions.remove(0)
}

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

/// Acceptance: my own PR in a screenshots Repo, state missing, would have
/// merged: Add screenshots, Waiting On me; Merge is withheld.
#[test]
fn my_pr_missing_screenshots_is_add_screenshots_not_merge() {
    let decision = decide_one(
        mine_ready(1).screenshots(Screenshots::Missing),
        &screenshots_in_app(),
    );
    assert_eq!(
        decision.next_step,
        Some(NextStep::AddScreenshots),
        "{decision:#?}"
    );
    assert_eq!(decision.waiting_on, WaitingOn::Me);
    assert!(
        said(&decision).contains("Screenshots"),
        "{}",
        said(&decision)
    );
}

/// With the state present or "No visual change", the PR merges as before.
#[test]
fn screenshots_or_no_visual_change_lets_it_merge() {
    for state in [Screenshots::Present, Screenshots::NoVisualChange] {
        let decision = decide_one(mine_ready(1).screenshots(state), &screenshots_in_app());
        assert_eq!(
            decision.next_step,
            Some(NextStep::Merge),
            "{state:?}: {decision:#?}"
        );
    }
}

/// Acceptance: only the viewer's own PRs, and only in Repos with the setting.
#[test]
fn others_prs_and_other_repos_are_never_held_for_screenshots() {
    let theirs = decide_one(
        pr(APP, 2)
            .author("alice")
            .approved()
            .passing("ci")
            .screenshots(Screenshots::Missing),
        &screenshots_in_app(),
    );
    assert_eq!(theirs.next_step, Some(NextStep::Merge), "{theirs:#?}");

    let unset = decide_one(
        mine_ready(3).screenshots(Screenshots::Missing),
        &Policies::default(),
    );
    assert_eq!(unset.next_step, Some(NextStep::Merge), "{unset:#?}");
}

/// Acceptance: placed after Address review and before Review.
#[test]
fn add_screenshots_comes_after_address_review_and_before_review() {
    let review = decide_one(
        pr(APP, 4)
            .author(ME)
            .changes_requested()
            .screenshots(Screenshots::Missing),
        &screenshots_in_app(),
    );
    assert_eq!(
        review.next_step,
        Some(NextStep::AddressReview),
        "{review:#?}"
    );
    assert!(
        said(&review).contains("Screenshots"),
        "also: {}",
        said(&review)
    );

    let requested = decide_one(
        pr(APP, 5)
            .author(ME)
            .review_requested_from("alice")
            .screenshots(Screenshots::Missing),
        &screenshots_in_app(),
    );
    assert_eq!(
        requested.next_step,
        Some(NextStep::AddScreenshots),
        "{requested:#?}"
    );
}

/// Acceptance: skipped, Add screenshots falls through, but Merge still needs
/// the state; the PR has no Next Step and says why.
#[test]
fn a_skipped_add_screenshots_never_unblocks_merge() {
    let policies = screenshots_in_app()
        .with_skips(Skips::default().with(RepoId::new("octo", "app"), [NextStep::AddScreenshots]));
    let decision = decide_one(mine_ready(6).screenshots(Screenshots::Missing), &policies);
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert_eq!(decision.not_tracked, Some(NextStep::AddScreenshots));
    let text = said(&decision);
    assert!(text.contains("Add screenshots is not tracked"), "{text}");
    assert!(text.contains("Screenshots"), "{text}");
}

/// A body den didn't read never yields Add screenshots or Merge: the state is
/// an unknown, named in the Explanation.
#[test]
fn an_unread_body_is_unknown_never_add_screenshots_or_merge() {
    let decision = decide_one(
        mine_ready(7).screenshots(Screenshots::NotRead),
        &screenshots_in_app(),
    );
    assert_eq!(decision.next_step, None, "{decision:#?}");
    assert!(
        decision
            .explanation
            .unknowns
            .iter()
            .any(|u| u.contains("Screenshots")),
        "{decision:#?}"
    );
}
