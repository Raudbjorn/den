//! Publication text from the same observed Review Points that gate merging.

use crate::{
    decide::points::{PointRef, review_points},
    domain::facts::PullRequest,
};

pub(super) fn render(pr: &PullRequest, viewer: &str, prefixes: &[String]) -> String {
    let points = review_points(pr.details.as_ref(), pr.author.as_deref(), viewer, prefixes);
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
