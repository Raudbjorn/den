//! Publication text from the same observed Review Points that gate merging.

use crate::{
    decide::points::{PointRef, PointState, ReviewPoints, review_points},
    domain::facts::PullRequest,
};
use serde::{Deserialize, Serialize};

/// One Pull Request's resolution comment (#78), as the plan carries it: the
/// sheet tallies it and asks again when it changes before launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// The Pull Request it is posted on.
    pub url: String,
    /// The comment, Markdown, exactly as `{resolutions}` carries it.
    pub body: String,
    /// For the sheet: how many Points, by state ("3 Points: 2 fixed, 1
    /// resolved").
    pub tally: String,
}

/// `pr`'s resolution comment, from the Review Points `decide` counts.
pub(super) fn of(pr: &PullRequest, viewer: &str, prefixes: &[String]) -> Resolution {
    let points = review_points(pr.details.as_ref(), pr.author.as_deref(), viewer, prefixes);
    Resolution {
        url: pr.url.clone(),
        body: render(pr, &points),
        tally: tally(&points),
    }
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

fn render(pr: &PullRequest, points: &ReviewPoints) -> String {
    let mut lines = vec![format!(
        "## Review resolutions for [{}#{}]({})",
        text(&pr.repo.to_string()),
        pr.number,
        pr.url
    )];
    for point in &points.points {
        let reference = match &point.reference {
            PointRef::Thread { id } => format!("thread {id}"),
            PointRef::Review { review, n: Some(n) } => format!("{review}.{n}"),
            PointRef::Review { review, n: None } => format!("review {review}"),
        };
        let reference = match &point.url {
            Some(url) => format!("[{}]({url})", text(&reference)),
            None => text(&reference),
        };
        let mut line = format!(
            "- {reference}; author: {}; state: {}",
            text(point.author.as_deref().unwrap_or("unknown")),
            point.state.label()
        );
        if let Some(commit) = &point.commit {
            line.push_str(&format!("; commit: {}", text(commit)));
        }
        if let Some(summary) = &point.summary {
            line.push_str(&format!("; {}", text(summary)));
        }
        lines.push(line);
    }
    if points.points.is_empty() && points.complete() {
        lines.push("- No Review Points were recorded.".to_owned());
    }
    for unread in &points.unread {
        lines.push(format!("- Not read: {}", text(unread)));
    }
    lines.join("\n")
}

// Keep a Job's summary on one line, with Markdown/HTML interpreted as text.
fn text(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
    {
        if matches!(
            ch,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
        if ch == '@' {
            // GitHub still links backslash-escaped mentions; break the token instead.
            escaped.push('\u{200b}');
        }
    }
    escaped
}
