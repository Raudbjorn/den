//! The resolution comment (#78, spec 0004 stories 50–53, ADR-0010): what
//! happened to every Review Point of a Pull Request, written by den from the
//! Points it observed and posted verbatim by the Merge or Land Job
//! immediately before it merges that Pull Request. Pure.
//!
//! One line per Point, in the order the detail lists them: its state (with
//! the commit when a marker names one), its reference linked to GitHub, its
//! reviewer's login, and the summary the viewer's marker kept, when there is
//! one. Nothing else: no reviewer-written text reaches the comment (story
//! 53), and logins are written without `@`, so posting it notifies no one.
//!
//! `{resolutions}` puts each comment in the prompt between `~~~` fences,
//! headed by the Pull Request it is for: one for a Merge Job, one per member
//! in landing order for a Land Job.

use crate::decide::points::{Point, PointRef, PointState, ReviewPoints};
use serde::{Deserialize, Serialize};

/// Characters of a commit the comment shows: GitHub links a commit
/// reference this long.
const SHORT_COMMIT_CHARS: usize = 7;
/// The fence around each comment in the prompt.
pub const FENCE: &str = "~~~";

/// One Pull Request's resolution comment, as the Job is told to post it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// The Pull Request it is posted on.
    pub url: String,
    /// The comment, Markdown, posted verbatim.
    pub body: String,
    /// For the sheet: how many Points, by state ("3 Points: 2 fixed, 1
    /// resolved").
    pub tally: String,
}

impl Resolution {
    /// The resolution comment for the Pull Request at `url`, whose head den
    /// saw at `head`, from its `points`.
    pub fn of(url: &str, head: &str, points: &ReviewPoints) -> Self {
        Self {
            url: url.to_owned(),
            body: body(head, points),
            tally: tally(points),
        }
    }
}

/// What `{resolutions}` is: each comment headed by its Pull Request and
/// fenced, in order.
pub fn placeholder(resolutions: &[Resolution]) -> String {
    resolutions
        .iter()
        .map(|r| {
            format!(
                "Resolution comment for {}:\n{FENCE}markdown\n{}\n{FENCE}",
                r.url, r.body
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The comment's Markdown.
fn body(head: &str, points: &ReviewPoints) -> String {
    let at = short(head);
    let mut lines = vec![if points.points.is_empty() {
        format!("**Review Points** at merge, as den observed them at {at}: none.")
    } else {
        format!("**Review Points** at merge, as den observed them at {at}:")
    }];
    if !points.points.is_empty() {
        lines.push(String::new());
        lines.extend(points.points.iter().map(line));
    }
    if !points.complete() {
        lines.push(String::new());
        lines.push(format!(
            "den couldn't read every Point: {}.",
            points.unread.join("; ")
        ));
    }
    lines.join("\n")
}

/// `- fixed in abc1234: [review 101.2](…) by alice — Renamed the flag`.
fn line(point: &Point) -> String {
    let state = match &point.commit {
        Some(commit) => format!("{} in {}", point.state.label(), short(commit)),
        None => point.state.label().to_owned(),
    };
    let which = match &point.reference {
        PointRef::Thread { .. } => "thread".to_owned(),
        PointRef::Review { review, n: None } => format!("review {review}"),
        PointRef::Review { review, n: Some(n) } => format!("review {review}.{n}"),
    };
    let which = match point.url.as_deref().filter(|url| linkable(url)) {
        Some(url) => format!("[{which}]({url})"),
        None => which,
    };
    let by = point
        .author
        .as_deref()
        .filter(|login| is_login(login))
        .unwrap_or("an unknown account");
    let summary = point
        .summary
        .as_deref()
        .map(one_line)
        .filter(|s| !s.is_empty())
        .map(|s| format!(" — {s}"))
        .unwrap_or_default();
    format!("- {state}: {which} by {by}{summary}")
}

/// "3 Points: 2 fixed, 1 resolved"; "no Review Points".
fn tally(points: &ReviewPoints) -> String {
    const ORDER: [PointState; 8] = [
        PointState::Open,
        PointState::NotYetSplit,
        PointState::NeedsDecision,
        PointState::Fixed,
        PointState::Declined,
        PointState::FollowUp,
        PointState::Answered,
        PointState::Resolved,
    ];
    let total = points.points.len();
    if total == 0 {
        return "no Review Points".to_owned();
    }
    let counts: Vec<String> = ORDER
        .iter()
        .filter_map(|state| {
            let n = points.points.iter().filter(|p| p.state == *state).count();
            (n > 0).then(|| format!("{n} {}", state.label()))
        })
        .collect();
    let noun = if total == 1 { "Point" } else { "Points" };
    format!("{total} {noun}: {}", counts.join(", "))
}

fn short(commit: &str) -> String {
    commit.chars().take(SHORT_COMMIT_CHARS).collect()
}

/// A URL den puts in a Markdown link as it is: GitHub's https, with nothing
/// that could end the link early.
fn linkable(url: &str) -> bool {
    url.starts_with("https://")
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '(' | ')' | '<' | '>'))
}

/// A GitHub login, a bot's included (`name[bot]`): nothing Markdown reads.
fn is_login(login: &str) -> bool {
    !login.is_empty()
        && login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '[' | ']'))
}

/// The summary on one line: control characters become spaces, runs of
/// whitespace one space.
fn one_line(summary: &str) -> String {
    summary
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
