//! A Pull Request's feedback facts (#75, spec 0004 "Facts", ADR-0010): its
//! reviews, its review threads and the markers in its comments, as GitHub
//! showed them. Nothing decided: [`crate::decide::points`] turns them into
//! Review Points.
//!
//! No text others wrote is kept. A review keeps whether its body is more than
//! a stock approval, never the body. A comment keeps only its marker lines,
//! parsed ([`parse_markers`]), and, on the viewer's own comments only, the one
//! line after each `den:point` marker as that Point's summary, cut to
//! [`MAX_SUMMARY_CHARS`].

use super::pull_request::{ActorKind, ReviewVerdict};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// The most characters of a Point's summary den keeps.
pub const MAX_SUMMARY_CHARS: usize = 200;
/// den's own marker prefix: `<!-- den:point … -->`. A legacy prefix may not
/// be it.
pub const DEN_MARKER_PREFIX: &str = "den";

/// One review, any state, without its body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRecord {
    /// GitHub's number for the review (`databaseId`), as markers name it and
    /// its URL ends (`#pullrequestreview-<id>`).
    pub id: u64,
    /// `None` when GitHub has no author (a deleted account).
    pub author: Option<String>,
    pub author_kind: ActorKind,
    pub verdict: ReviewVerdict,
    /// `None` for a review not yet submitted (only its author sees those).
    pub submitted_at: Option<Timestamp>,
    pub url: String,
    /// The body is more than a stock approval ("LGTM", "ship it", 👍)
    /// once HTML comments are taken out ([`is_stock_approval`]).
    pub has_body: bool,
}

/// One review thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRecord {
    /// GitHub's node ID for the thread.
    pub id: String,
    /// Its first comment's URL; `None` when GitHub returned none.
    pub url: Option<String>,
    /// Its first comment's author.
    pub author: Option<String>,
    pub resolved: bool,
}

/// One comment on the Pull Request that carries markers: who posted it, when,
/// where, and the markers. Comments without markers are not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerComment {
    /// `None` when GitHub has no author (a deleted account).
    pub author: Option<String>,
    pub created_at: Timestamp,
    pub url: String,
    pub markers: Vec<Marker>,
}

/// A marker line, parsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Marker {
    /// `<!-- den:point <review id>.<n> <state> [<commit>] -->`.
    Point {
        review: u64,
        /// The Point's number within its review, from 1.
        n: u32,
        state: MarkedState,
        commit: Option<String>,
        /// The line after the marker, on the viewer's own comments only.
        summary: Option<String>,
    },
    /// `<!-- <prefix>:review-notes:<review id>:addressed|needs-decision -->`:
    /// every Point of that review. Which prefixes count is configuration
    /// (`legacy_marker_prefixes`); the facts keep any well-formed one.
    Legacy {
        prefix: String,
        review: u64,
        state: LegacyState,
    },
}

/// The states a `den:point` marker can set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkedState {
    Open,
    Fixed,
    Declined,
    FollowUp,
    NeedsDecision,
    Answered,
}

impl MarkedState {
    /// As markers spell it.
    pub fn word(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Fixed => "fixed",
            Self::Declined => "declined",
            Self::FollowUp => "follow-up",
            Self::NeedsDecision => "needs-decision",
            Self::Answered => "answered",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        [
            Self::Open,
            Self::Fixed,
            Self::Declined,
            Self::FollowUp,
            Self::NeedsDecision,
            Self::Answered,
        ]
        .into_iter()
        .find(|state| state.word() == word)
    }
}

/// The states a legacy `review-notes` marker can set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyState {
    Addressed,
    NeedsDecision,
}

/// A prefix as `legacy_marker_prefixes` allows it: lowercase ASCII letters,
/// digits and `-`, not empty.
pub fn is_marker_prefix(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The markers in a comment's body, in order. A marker is a line that is
/// exactly the marker (surrounding spaces aside). With `keep_summaries`
/// (the viewer's own comment), a `den:point` marker keeps the next line as
/// its summary when that line is text and not another marker.
pub fn parse_markers(body: &str, keep_summaries: bool) -> Vec<Marker> {
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| {
            let mut marker = parse_marker(line)?;
            if let Marker::Point { summary, .. } = &mut marker
                && keep_summaries
            {
                *summary = lines
                    .get(i + 1)
                    .filter(|next| !next.is_empty() && parse_marker(next).is_none())
                    .map(|next| next.chars().take(MAX_SUMMARY_CHARS).collect());
            }
            Some(marker)
        })
        .collect()
}

/// One marker line, or `None` when `line` isn't exactly a marker.
fn parse_marker(line: &str) -> Option<Marker> {
    let inner = line.strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    if let Some(rest) = inner.strip_prefix("den:point ") {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let (reference, state, commit) = match words.as_slice() {
            [reference, state] => (*reference, *state, None),
            [reference, state, commit] => (*reference, *state, Some(*commit)),
            _ => return None,
        };
        let (review, n) = reference.split_once('.')?;
        let n: u32 = digits(n)?.try_into().ok()?;
        if n == 0 {
            return None;
        }
        let commit = match commit {
            Some(commit) if is_commit(commit) => Some(commit.to_ascii_lowercase()),
            Some(_) => return None,
            None => None,
        };
        return Some(Marker::Point {
            review: digits(review)?,
            n,
            state: MarkedState::parse(state)?,
            commit,
            summary: None,
        });
    }
    let (prefix, rest) = inner.split_once(":review-notes:")?;
    let (review, state) = rest.split_once(':')?;
    if !is_marker_prefix(prefix) {
        return None;
    }
    let state = match state {
        "addressed" => LegacyState::Addressed,
        "needs-decision" => LegacyState::NeedsDecision,
        _ => return None,
    };
    Some(Marker::Legacy {
        prefix: prefix.to_owned(),
        review: digits(review)?,
        state,
    })
}

/// A positive number written in ASCII digits only.
fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// An abbreviated or full commit SHA: 7 to 40 hex digits.
fn is_commit(text: &str) -> bool {
    (7..=40).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Phrases that are only an approval, compared after [`normalise`].
const STOCK_APPROVALS: [&str; 12] = [
    "",
    "lgtm",
    "looksgood",
    "looksgoodtome",
    "shipit",
    "approved",
    "approve",
    "+1",
    "👍",
    "lgtm👍",
    "🚀",
    "🚢",
];

/// The body is only a stock approval (or nothing) once HTML comments are
/// taken out: "LGTM", "Looks good to me!", "ship it", 👍.
pub fn is_stock_approval(body: &str) -> bool {
    STOCK_APPROVALS.contains(&normalise(&without_html_comments(body)).as_str())
}

/// Lowercase, without whitespace, `.`, `!`, `,` or the `:+1:` shortcode's colons.
fn normalise(text: &str) -> String {
    text.replace(":+1:", "+1")
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '.' | '!' | ','))
        .flat_map(char::to_lowercase)
        .collect()
}

/// `text` without `<!-- … -->` comments; an unclosed one runs to the end.
pub fn without_html_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn den_point_markers_parse_with_and_without_a_commit() {
        let body = "Addressed:\n<!-- den:point 123.1 fixed abc1234 -->\nRenamed the flag\n  <!-- den:point 123.2 needs-decision -->  \n\n<!-- den:point 123.3 declined -->\n<!-- den:point 123.4 follow-up -->\nnext";
        let markers = parse_markers(body, true);
        assert_eq!(
            markers,
            [
                Marker::Point {
                    review: 123,
                    n: 1,
                    state: MarkedState::Fixed,
                    commit: Some("abc1234".to_owned()),
                    summary: Some("Renamed the flag".to_owned()),
                },
                Marker::Point {
                    review: 123,
                    n: 2,
                    state: MarkedState::NeedsDecision,
                    commit: None,
                    summary: None,
                },
                Marker::Point {
                    review: 123,
                    n: 3,
                    state: MarkedState::Declined,
                    commit: None,
                    // The next line is another marker: no summary.
                    summary: None,
                },
                Marker::Point {
                    review: 123,
                    n: 4,
                    state: MarkedState::FollowUp,
                    commit: None,
                    summary: Some("next".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn summaries_are_kept_only_when_asked_and_cut() {
        let long = "x".repeat(MAX_SUMMARY_CHARS + 50);
        let body = format!("<!-- den:point 9.1 fixed -->\n{long}");
        let Marker::Point { summary, .. } = &parse_markers(&body, true)[0] else {
            panic!("a point");
        };
        assert_eq!(
            summary.as_ref().map(|s| s.chars().count()),
            Some(MAX_SUMMARY_CHARS)
        );
        let Marker::Point { summary, .. } = &parse_markers(&body, false)[0] else {
            panic!("a point");
        };
        assert_eq!(summary, &None);
    }

    #[test]
    fn legacy_markers_parse_for_any_well_formed_prefix() {
        assert_eq!(
            parse_markers("<!-- acme-bot2:review-notes:77:needs-decision -->", false),
            [Marker::Legacy {
                prefix: "acme-bot2".to_owned(),
                review: 77,
                state: LegacyState::NeedsDecision,
            }]
        );
        assert_eq!(
            parse_markers("<!--x:review-notes:5:addressed-->", false),
            [Marker::Legacy {
                prefix: "x".to_owned(),
                review: 5,
                state: LegacyState::Addressed,
            }]
        );
    }

    #[test]
    fn anything_else_is_no_marker() {
        for line in [
            "<!-- den:point 123 fixed -->",
            "<!-- den:point 123.0 fixed -->",
            "<!-- den:point 12a.1 fixed -->",
            "<!-- den:point 123.1 done -->",
            "<!-- den:point 123.1 fixed nothex! -->",
            "<!-- den:point 123.1 fixed abc -->",
            "<!-- den:point 123.1 fixed abc1234 extra -->",
            "see <!-- den:point 123.1 fixed -->",
            "<!-- Acme:review-notes:5:addressed -->",
            "<!-- acme:review-notes:5:done -->",
            "<!-- acme:review-notes:-5:addressed -->",
            "<!-- den:point 123.1 fixed",
        ] {
            assert!(parse_markers(line, true).is_empty(), "{line}");
        }
    }

    #[test]
    fn stock_approvals_are_recognised_and_html_comments_ignored() {
        for body in [
            "",
            "LGTM",
            "lgtm!",
            "Looks good to me.",
            "Ship it!",
            "👍",
            ":+1:",
            "  \n",
            "<!-- template: describe your review -->\nLGTM",
        ] {
            assert!(is_stock_approval(body), "{body:?}");
        }
        for body in [
            "LGTM, but please rename the flag before merging",
            "Before merging, please add a test.",
            "nit: typo",
        ] {
            assert!(!is_stock_approval(body), "{body:?}");
        }
    }

    #[test]
    fn html_comments_are_taken_out_even_unclosed() {
        assert_eq!(without_html_comments("a<!-- b -->c<!-- d"), "ac");
    }
}
