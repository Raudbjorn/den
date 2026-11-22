//! The detail's REVIEW POINTS section (#77, spec 0004 stories 43, 46, 48):
//! each Point of the selected Pull Request with its state, reviewer, link
//! and the commit when fixed; markers others posted, shown as not counted;
//! and, for needs-decision Points, the marker lines `m` copies.

use super::State;
use crate::{
    app::view_model::{PointLine, PointTone, ReviewPointsDetail, SEPARATOR},
    decide::points::{self, Point, PointRef, PointState, ReviewPoints},
    domain::{Observation, WorkItemId},
    ordering::same_work_item,
};

const LABEL: &str = "REVIEW POINTS";
/// Characters of a fixed Point's commit shown.
const SHORT_COMMIT_CHARS: usize = 7;

/// The Review Points of `id`, when it is a Pull Request in `observation`
/// ([`points::of`], which the Address review Job's `{review_notes}` uses too).
pub fn of(
    observation: Option<&Observation>,
    prefixes: &[String],
    id: &WorkItemId,
) -> Option<ReviewPoints> {
    points::of(observation?, prefixes, id)
}

/// The selected Work Item's Review Points, when it is a Pull Request.
pub fn selected(state: &State<'_>) -> Option<ReviewPoints> {
    let decision = state.decisions.get(state.selected?)?;
    of(
        state.observation,
        state.legacy_marker_prefixes,
        &decision.item.id,
    )
}

/// The section for the selected Work Item, with what the last `m` on it did.
pub fn selected_detail(state: &State<'_>, id: &WorkItemId) -> Option<ReviewPointsDetail> {
    let points = of(state.observation, state.legacy_marker_prefixes, id)?;
    let copied = state
        .copied
        .filter(|(item, _)| same_work_item(item, id))
        .map(|(_, outcome)| outcome);
    detail(&points, copied)
}

/// The marker lines settling every needs-decision Point as answered, one per
/// line, for the clipboard; `None` when no Point needs a decision.
pub fn decision_markers(points: &ReviewPoints) -> Option<String> {
    let lines: Vec<String> = points
        .points
        .iter()
        .filter(|p| p.state == PointState::NeedsDecision)
        .filter_map(Point::answering_marker)
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The section; `None` with no Points and nothing unread. `copied` is what
/// the last `m` on this Work Item did, shown while a Point still needs a
/// decision (once settled, the copy is history).
pub fn detail(
    points: &ReviewPoints,
    copied: Option<&Result<(), String>>,
) -> Option<ReviewPointsDetail> {
    if points.points.is_empty() && points.unread.is_empty() {
        return None;
    }
    let decisions = points
        .points
        .iter()
        .filter(|p| p.state == PointState::NeedsDecision)
        .count();
    Some(ReviewPointsDetail {
        label: LABEL.to_owned(),
        lines: points.points.iter().map(line).collect(),
        unread: points.unread.clone(),
        settle: (decisions > 0).then(|| {
            let lines = if decisions == 1 { "its marker line" } else { "their marker lines" };
            format!(
                "m copies {lines} (answered) for you to post with your decision on GitHub; den posts nothing"
            )
        }),
        copied: copied.filter(|_| decisions > 0).map(|outcome| match outcome {
            Ok(()) => "copied: paste it into your reply on GitHub".to_owned(),
            Err(why) => format!("couldn't copy: {why}"),
        }),
    })
}

/// "needs decision · review 101.2 by alice · Which API? · https://…".
fn line(point: &Point) -> PointLine {
    let which = match &point.reference {
        PointRef::Thread { .. } => "thread".to_owned(),
        PointRef::Review { review, n: None } => format!("review {review}"),
        PointRef::Review { review, n: Some(n) } => format!("review {review}.{n}"),
    };
    let mut parts = vec![
        point.state.label().to_owned(),
        format!(
            "{which} by {}",
            point.author.as_deref().unwrap_or("an unknown account")
        ),
    ];
    if let Some(commit) = &point.commit {
        parts.push(format!(
            "in {}",
            commit.chars().take(SHORT_COMMIT_CHARS).collect::<String>()
        ));
    }
    if let Some(summary) = &point.summary {
        parts.push(summary.clone());
    }
    if let Some(url) = point.set_by.as_ref().or(point.url.as_ref()) {
        parts.push(url.clone());
    }
    PointLine {
        text: parts.join(&format!(" {SEPARATOR} ")),
        tone: match point.state {
            PointState::Open | PointState::NotYetSplit => PointTone::Blocking,
            PointState::NeedsDecision => PointTone::Yours,
            PointState::Resolved
            | PointState::Fixed
            | PointState::Declined
            | PointState::FollowUp
            | PointState::Answered => PointTone::Settled,
        },
        ignored: point
            .ignored_markers
            .iter()
            .map(|marker| {
                format!(
                    "marker by {} not counted (only yours count): {}",
                    marker.author.as_deref().unwrap_or("an unknown account"),
                    marker.url
                )
            })
            .collect(),
    }
}
