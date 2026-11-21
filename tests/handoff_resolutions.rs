//! The resolution comment's rendering (#78), pure: what den puts in a
//! comment the Job posts verbatim, from Points whose logins, URLs and
//! summaries came from GitHub. Nothing in them can add a line, a link or a
//! mention; Points den couldn't read in full say so.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    decide::points::{Point, PointRef, PointState, ReviewPoints},
    handoff::resolutions::{Resolution, placeholder},
};

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const URL: &str = "https://github.com/acme/api/pull/4";

fn point(state: PointState) -> Point {
    Point {
        reference: PointRef::Review {
            review: 7,
            n: Some(1),
        },
        author: Some("alice".to_owned()),
        url: Some(format!("{URL}#pullrequestreview-7")),
        state,
        commit: None,
        summary: None,
        set_by: None,
        ignored_markers: Vec::new(),
    }
}

fn body(points: Vec<Point>) -> String {
    Resolution::of(
        URL,
        HEAD,
        &ReviewPoints {
            points,
            unread: Vec::new(),
        },
    )
    .body
}

#[test]
fn a_login_that_isnt_one_is_an_unknown_account() {
    for hostile in ["x](https://evil.example)", "@team", "a b", ""] {
        let mut p = point(PointState::Fixed);
        p.author = Some(hostile.to_owned());
        let text = body(vec![p]);
        assert!(text.contains("by an unknown account"), "{hostile}: {text}");
        assert!(!text.contains("evil.example"), "{text}");
    }
    let mut bot = point(PointState::Resolved);
    bot.author = Some("review-bot[bot]".to_owned());
    assert!(body(vec![bot]).contains("by review-bot[bot]"));
}

#[test]
fn a_url_that_could_end_the_link_early_is_left_unlinked() {
    for hostile in [
        "https://github.com/a) [x](https://evil.example",
        "http://github.com/plain-http",
        "https://github.com/with space",
    ] {
        let mut p = point(PointState::Fixed);
        p.url = Some(hostile.to_owned());
        let text = body(vec![p]);
        assert!(text.contains("- fixed: review 7.1 by alice"), "{text}");
        assert!(!text.contains("]("), "{text}");
    }
}

#[test]
fn a_summary_stays_on_its_line() {
    let mut p = point(PointState::Fixed);
    p.commit = Some("abcdef1234567890".to_owned());
    p.summary = Some("Renamed\tthe\u{7}flag\r\n- injected line".to_owned());
    let text = body(vec![p]);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.last().copied(),
        Some(
            "- fixed in abcdef1: [review 7.1](https://github.com/acme/api/pull/4#pullrequestreview-7) by alice — Renamed the flag - injected line"
        ),
        "{text}"
    );
}

#[test]
fn points_den_couldnt_read_in_full_say_so() {
    let resolution = Resolution::of(
        URL,
        HEAD,
        &ReviewPoints {
            points: Vec::new(),
            unread: vec!["not every one of its comments was read".to_owned()],
        },
    );
    assert_eq!(
        resolution.body,
        "**Review Points** at merge, as den observed them at 0123456: none.\n\nden couldn't read every Point: not every one of its comments was read."
    );
}

#[test]
fn the_tally_counts_points_by_state() {
    let resolution = Resolution::of(
        URL,
        HEAD,
        &ReviewPoints {
            points: vec![
                point(PointState::Resolved),
                point(PointState::Fixed),
                point(PointState::Resolved),
            ],
            unread: Vec::new(),
        },
    );
    assert_eq!(resolution.tally, "3 Points: 1 fixed, 2 resolved");
}

#[test]
fn the_placeholder_fences_each_comment_under_its_pull_request() {
    let first = Resolution::of(URL, HEAD, &ReviewPoints::default());
    let second = Resolution::of(
        "https://github.com/acme/app/pull/2",
        HEAD,
        &ReviewPoints::default(),
    );
    assert_eq!(
        placeholder(&[first.clone(), second.clone()]),
        format!(
            "Resolution comment for {URL}:\n~~~markdown\n{}\n~~~\n\nResolution comment for https://github.com/acme/app/pull/2:\n~~~markdown\n{}\n~~~",
            first.body, second.body
        )
    );
    assert_eq!(placeholder(&[]), "");
}
