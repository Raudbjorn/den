//! Seam 2, REVIEW POINTS in the detail (#77, spec 0004 stories 43, 46, 48):
//! a Pull Request's detail lists each Point with its state, reviewer, link
//! and the commit when fixed; markers posted by anyone but the viewer show
//! with their author, as not counted; and `m` on a needs-decision Point puts
//! its answering marker line on the clipboard (den posts nothing, ADR-0002),
//! with the key bar showing `m` only then. Asserts on the ViewModel and the
//! fake launcher's clipboard.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Event, Key,
        view_model::{PointTone, ReviewPointsDetail, ViewModel},
    },
    domain::{NextStep, facts::ReviewVerdict},
};
use support::{
    build::{ME, ObservationBuilder, PrBuilder, pr},
    harness::Harness,
};

const ALICE: &str = "alice";
const BOB: &str = "bob";
const T1: &str = "2026-11-10T10:00:00Z";
const T2: &str = "2026-11-11T10:00:00Z";
const T3: &str = "2026-11-12T10:00:00Z";

/// My PR with checks passing, approved.
fn mine(number: u64) -> PrBuilder {
    pr("acme/app", number).author(ME).passing("ci").approved()
}

/// den started over an Observation of `builder` alone, which is selected.
fn started(builder: PrBuilder) -> Harness {
    let mut harness = Harness::with_owners(&["acme"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/app")
        .pull_request(builder)
        .build()));
    harness.start();
    harness
}

fn section(view: &ViewModel) -> ReviewPointsDetail {
    view.detail
        .content
        .as_ref()
        .expect("the PR is selected")
        .review_points
        .clone()
        .expect("a REVIEW POINTS section")
}

fn hint_keys(view: &ViewModel) -> Vec<String> {
    view.footer.hints.iter().map(|h| h.key.clone()).collect()
}

/// alice's review 100 split into four Points I settled, and bob's thread.
fn settled_and_open() -> PrBuilder {
    mine(1)
        .review_body(100, ALICE, ReviewVerdict::Commented, T1, true)
        .comment(
            1,
            ME,
            T2,
            "<!-- den:point 100.1 fixed abc1234def -->\nRenamed the flag\n\
             <!-- den:point 100.2 declined -->\nOut of scope\n\
             <!-- den:point 100.3 follow-up -->\nFiled\n\
             <!-- den:point 100.4 answered -->\nExplained",
        )
        .thread("T_1", BOB, false)
}

#[test]
fn the_detail_lists_each_point_with_state_reviewer_commit_and_link() {
    let harness = started(settled_and_open());
    let view = &harness.view;
    assert_eq!(
        view.detail.content.as_ref().unwrap().next_step,
        NextStep::AddressReview.label(),
        "the open thread holds it"
    );
    let points = section(view);
    assert_eq!(points.label, "REVIEW POINTS");
    let texts: Vec<&str> = points.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts.len(), 5, "{texts:#?}");

    // The fixed Point: state, which Point, reviewer, short commit, summary,
    // and the comment that set it.
    let fixed = texts
        .iter()
        .find(|t| t.starts_with("fixed"))
        .expect("the fixed Point");
    assert!(fixed.contains("review 100.1 by alice"), "{fixed}");
    assert!(fixed.contains("in abc1234 "), "{fixed}");
    assert!(
        !fixed.contains("abc1234def"),
        "the commit is shortened: {fixed}"
    );
    assert!(fixed.contains("Renamed the flag"), "{fixed}");
    assert!(
        fixed.contains("https://github.com/acme/app/pull/1#issuecomment-1"),
        "{fixed}"
    );
    for state in ["declined", "follow-up", "answered"] {
        assert!(
            texts.iter().any(|t| t.starts_with(state)),
            "{state}: {texts:#?}"
        );
    }

    // The open thread: its reviewer and its link, and it blocks.
    let thread = points
        .lines
        .iter()
        .find(|l| l.text.starts_with("open"))
        .expect("the open thread");
    assert!(thread.text.contains("thread by bob"), "{}", thread.text);
    assert!(
        thread
            .text
            .contains("https://github.com/acme/app/pull/1#discussion_T_1"),
        "{}",
        thread.text
    );
    assert_eq!(thread.tone, PointTone::Blocking);
    assert!(
        points
            .lines
            .iter()
            .filter(|l| !l.text.starts_with("open"))
            .all(|l| l.tone == PointTone::Settled)
    );
    // Nothing needs a decision: no settle line, no `m`.
    assert_eq!(points.settle, None);
    assert!(!hint_keys(view).contains(&"m".to_owned()));
}

#[test]
fn a_marker_by_someone_else_shows_with_its_author_as_not_counted() {
    let harness = started(
        mine(1)
            .review_body(100, ALICE, ReviewVerdict::Commented, T1, true)
            .comment(
                5,
                BOB,
                T2,
                "<!-- den:point 100.1 fixed abc1234 -->\nI fixed it for you",
            ),
    );
    let points = section(&harness.view);
    assert_eq!(points.lines.len(), 1, "{points:#?}");
    let line = &points.lines[0];
    // bob's marker didn't settle it.
    assert!(line.text.starts_with("not yet split"), "{}", line.text);
    assert_eq!(line.tone, PointTone::Blocking);
    assert_eq!(
        line.ignored,
        ["marker by bob not counted (only yours count): \
          https://github.com/acme/app/pull/1#issuecomment-5"]
    );
}

/// Someone else's PR whose review 100 has a Point I asked about.
fn asking() -> PrBuilder {
    pr("acme/app", 1)
        .author(BOB)
        .passing("ci")
        .approved()
        .review_body(100, ALICE, ReviewVerdict::Commented, T1, true)
        .comment(
            7,
            ME,
            T2,
            "<!-- den:point 100.1 needs-decision -->\nKeep the old flag?\n\
             <!-- den:point 100.2 needs-decision -->\nRename it?\n\
             <!-- den:point 100.3 fixed abc1234 -->",
        )
}

#[test]
fn m_copies_the_answering_marker_lines_of_the_points_needing_a_decision() {
    let mut harness = started(asking());
    let points = section(&harness.view);
    let yours: Vec<&str> = points
        .lines
        .iter()
        .filter(|l| l.tone == PointTone::Yours)
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(yours.len(), 2, "{points:#?}");
    assert!(yours[0].starts_with("needs decision"), "{}", yours[0]);
    assert!(yours[0].contains("Keep the old flag?"), "{}", yours[0]);
    assert_eq!(
        points.settle.as_deref(),
        Some(
            "m copies their marker lines (answered) for you to post with your decision \
             on GitHub; den posts nothing"
        )
    );
    assert!(hint_keys(&harness.view).contains(&"m".to_owned()));
    assert_eq!(points.copied, None);

    let effects = harness.key(Key::Char('m'));
    let [Effect::CopyToClipboard { text, .. }] = effects.as_slice() else {
        panic!("expected one copy: {effects:#?}");
    };
    assert_eq!(
        text,
        "<!-- den:point 100.1 answered -->\n<!-- den:point 100.2 answered -->"
    );
    for effect in &effects {
        harness.perform(effect);
    }
    assert_eq!(harness.launcher.clipboard(), std::slice::from_ref(text));
    assert_eq!(
        section(&harness.view).copied.as_deref(),
        Some("copied: paste it into your reply on GitHub")
    );
}

#[test]
fn a_copy_that_fails_says_why() {
    let mut harness = started(asking());
    let item = harness
        .key(Key::Char('m'))
        .into_iter()
        .find_map(|e| match e {
            Effect::CopyToClipboard { item, .. } => Some(item),
            _ => None,
        })
        .expect("a copy");
    harness.send(Event::Copied {
        item,
        outcome: Err("no clipboard tool found".to_owned()),
    });
    assert_eq!(
        section(&harness.view).copied.as_deref(),
        Some("couldn't copy: no clipboard tool found")
    );
}

#[test]
fn m_does_nothing_without_a_point_needing_a_decision() {
    // Points, none needing a decision.
    let mut harness = started(settled_and_open());
    assert!(harness.key(Key::Char('m')).is_empty());
    assert!(harness.launcher.clipboard().is_empty());

    // A PR without feedback has no section at all.
    let mut quiet = started(mine(2));
    assert_eq!(
        quiet.view.detail.content.as_ref().unwrap().review_points,
        None
    );
    assert!(quiet.key(Key::Char('m')).is_empty());
    assert!(!hint_keys(&quiet.view).contains(&"m".to_owned()));
}

#[test]
fn once_answered_the_point_settles_and_the_copy_note_goes() {
    let mut harness = started(asking());
    for effect in harness.key(Key::Char('m')) {
        harness.perform(&effect);
    }
    // I posted the lines; the next survey reads them.
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/app")
        .pull_request(asking().comment(
            8,
            ME,
            T3,
            "<!-- den:point 100.1 answered -->\n<!-- den:point 100.2 answered -->",
        ))
        .build()));
    harness.settle(Event::Key(Key::Char('r')));
    let points = section(&harness.view);
    assert!(
        points.lines.iter().all(|l| l.tone == PointTone::Settled),
        "{points:#?}"
    );
    assert_eq!(points.settle, None);
    assert_eq!(points.copied, None);
    assert!(!hint_keys(&harness.view).contains(&"m".to_owned()));
}
