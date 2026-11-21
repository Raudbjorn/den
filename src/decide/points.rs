//! Review Points (#75, ADR-0010, `CONTEXT.md`): everything a reviewer asked
//! for on a Pull Request, and where it stands. Pure: from a PR's feedback
//! facts, the viewer's login and the configured `legacy_marker_prefixes`.
//!
//! - Every review thread is a Point: open, or resolved (GitHub's state).
//! - Every review whose body is more than a stock approval, by someone other
//!   than the PR's author and not a bot, gives Points. Until a marker from
//!   the viewer names its Points (`<!-- den:point <review>.<n> <state> -->`),
//!   the whole body is one Point, **not yet split**.
//! - A legacy marker (`<!-- <prefix>:review-notes:<review>:addressed|needs-decision -->`)
//!   of a configured prefix sets every Point of its review: addressed is
//!   answered.
//! - Markers apply in the order their comments were posted; a later one wins.
//! - Only markers posted by the viewer change a state. Anyone else's are kept
//!   on the Points of the review they name, with their author, and change
//!   nothing: a marker gates merging, so it is a permission.
//! - A later approval by the same reviewer makes that reviewer's earlier
//!   open or not-yet-split review-body Points answered; one needing a
//!   decision stays so, and so does a state a marker set after the approval
//!   (a Point the viewer reopens after it stays open).
//!
//! Open, not yet split and needs decision **block**: they hold a PR at
//! Address review and withhold Merge (and so Land). Points den couldn't read
//! in full make the coverage partial, which withholds Merge with an unknown.

use crate::domain::{
    facts::{
        ActorKind, LegacyState, MarkedState, Marker, MarkerComment, PrDetails, ReviewRecord,
        ReviewVerdict,
    },
    ids::same_login,
};
use jiff::Timestamp;

/// Where a Point's state stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointState {
    /// A thread not resolved, or a body Point a marker says is open.
    Open,
    /// A review body no marker has split into Points yet.
    NotYetSplit,
    /// A thread resolved on GitHub.
    Resolved,
    Fixed,
    Declined,
    /// Out of scope for this Pull Request.
    FollowUp,
    /// The user's to settle.
    NeedsDecision,
    /// The reviewer approved again after raising it, or a legacy marker said
    /// it was addressed.
    Answered,
}

impl PointState {
    /// Holds the PR at Address review and withholds Merge.
    pub fn blocks(self) -> bool {
        matches!(self, Self::Open | Self::NotYetSplit | Self::NeedsDecision)
    }

    /// As den shows it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::NotYetSplit => "not yet split",
            Self::Resolved => "resolved",
            Self::Fixed => "fixed",
            Self::Declined => "declined",
            Self::FollowUp => "follow-up",
            Self::NeedsDecision => "needs decision",
            Self::Answered => "answered",
        }
    }

    fn marked(state: MarkedState) -> Self {
        match state {
            MarkedState::Open => Self::Open,
            MarkedState::Fixed => Self::Fixed,
            MarkedState::Declined => Self::Declined,
            MarkedState::FollowUp => Self::FollowUp,
            MarkedState::NeedsDecision => Self::NeedsDecision,
            MarkedState::Answered => Self::Answered,
        }
    }

    fn legacy(state: LegacyState) -> Self {
        match state {
            LegacyState::Addressed => Self::Answered,
            LegacyState::NeedsDecision => Self::NeedsDecision,
        }
    }
}

/// Which Point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointRef {
    /// A review thread, by GitHub's node ID.
    Thread { id: String },
    /// A review body's Point `<review>.<n>`; `n` is `None` for a body not
    /// yet split, whose one Point is the whole body.
    Review { review: u64, n: Option<u32> },
}

/// A marker den shows but doesn't apply: someone other than the viewer
/// posted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignMarker {
    pub author: Option<String>,
    pub url: String,
}

/// One Review Point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub reference: PointRef,
    /// The reviewer: the review's author, or the thread's first commenter.
    pub author: Option<String>,
    /// The review's or the thread's URL.
    pub url: Option<String>,
    pub state: PointState,
    /// The commit a `fixed` marker names.
    pub commit: Option<String>,
    /// The viewer's one-line summary kept with the marker that set the state.
    pub summary: Option<String>,
    /// The comment (or approving review) that set the state; `None` for
    /// GitHub's own thread state and for a body not yet split.
    pub set_by: Option<String>,
    /// Markers naming this Point's review that others posted (shown, never
    /// applied).
    pub ignored_markers: Vec<ForeignMarker>,
}

impl Point {
    /// The marker line that settles this Point as answered, for the user to
    /// post with their decision (#77): `<!-- den:point <review>.<n> answered -->`.
    /// A body not yet split is its Point 1. `None` for a thread, which GitHub
    /// settles by resolving it.
    pub fn answering_marker(&self) -> Option<String> {
        match self.reference {
            PointRef::Review { review, n } => Some(format!(
                "<!-- den:point {review}.{} answered -->",
                n.unwrap_or(1)
            )),
            PointRef::Thread { .. } => None,
        }
    }
}

/// A Pull Request's Review Points and what den couldn't read of them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewPoints {
    pub points: Vec<Point>,
    /// What den didn't read in full; empty when the Points are complete.
    pub unread: Vec<String>,
}

impl ReviewPoints {
    pub fn complete(&self) -> bool {
        self.unread.is_empty()
    }

    /// The review-body Points that block, in order. Thread Points are left
    /// to the unresolved-thread rule, which already counts them.
    pub fn blocking_bodies(&self) -> impl Iterator<Item = &Point> {
        self.points
            .iter()
            .filter(|p| matches!(p.reference, PointRef::Review { .. }) && p.state.blocks())
    }
}

/// The Review Points of a Pull Request by `pr_author`, from `details`
/// (`None`: not read, so nothing is known), for `viewer`, applying legacy
/// markers of `prefixes` only.
pub fn review_points(
    details: Option<&PrDetails>,
    pr_author: Option<&str>,
    viewer: &str,
    prefixes: &[String],
) -> ReviewPoints {
    let Some(details) = details else {
        return ReviewPoints {
            points: Vec::new(),
            unread: vec!["its reviews, threads and comments could not be read".to_owned()],
        };
    };
    let unread = [
        (details.reviews.complete, "reviews"),
        (details.threads.complete, "review threads"),
        (details.marker_comments.complete, "comments"),
    ]
    .into_iter()
    .filter(|(complete, _)| !complete)
    .map(|(_, what)| format!("not every one of its {what} was read"))
    .collect();
    let mut comments: Vec<&MarkerComment> = details.marker_comments.items.iter().collect();
    comments.sort_by_key(|c| c.created_at);
    let reviews = &details.reviews.items;
    let mut points: Vec<Point> = reviews
        .iter()
        .filter(|r| gives_points(r, pr_author))
        .flat_map(|review| body_points(review, reviews, &comments, viewer, prefixes))
        .collect();
    points.extend(details.threads.items.iter().map(|thread| Point {
        reference: PointRef::Thread {
            id: thread.id.clone(),
        },
        author: thread.author.clone(),
        url: thread.url.clone(),
        state: if thread.resolved {
            PointState::Resolved
        } else {
            PointState::Open
        },
        commit: None,
        summary: None,
        set_by: None,
        ignored_markers: Vec::new(),
    }));
    ReviewPoints { points, unread }
}

/// A review gives Points: a submitted opinion with more than a stock
/// approval in its body, by someone other than the PR's author, not a bot.
fn gives_points(review: &ReviewRecord, pr_author: Option<&str>) -> bool {
    let by_author = match (&review.author, pr_author) {
        (Some(reviewer), Some(author)) => same_login(reviewer, author),
        _ => false,
    };
    review.has_body
        && review.submitted_at.is_some()
        && matches!(
            review.verdict,
            ReviewVerdict::Approved | ReviewVerdict::ChangesRequested | ReviewVerdict::Commented
        )
        && review.author_kind != ActorKind::Bot
        && !by_author
}

/// One body Point as the markers left it.
#[derive(Clone)]
struct Marked {
    state: PointState,
    commit: Option<String>,
    summary: Option<String>,
    set_by: Option<String>,
    /// When the marker that set the state was posted; `None` for a body not
    /// yet split.
    set_at: Option<Timestamp>,
}

/// `review`'s Points: one per number the viewer's markers name, else the
/// whole body, not yet split.
fn body_points(
    review: &ReviewRecord,
    reviews: &[ReviewRecord],
    comments: &[&MarkerComment],
    viewer: &str,
    prefixes: &[String],
) -> Vec<Point> {
    let mut whole = Marked {
        state: PointState::NotYetSplit,
        commit: None,
        summary: None,
        set_by: None,
        set_at: None,
    };
    let mut split: Vec<(u32, Marked)> = Vec::new();
    let mut ignored = Vec::new();
    for comment in comments {
        let mine = comment
            .author
            .as_deref()
            .is_some_and(|a| same_login(a, viewer));
        for marker in &comment.markers {
            let applies = match marker {
                Marker::Point { review: id, .. } => *id == review.id,
                Marker::Legacy {
                    prefix, review: id, ..
                } => *id == review.id && prefixes.iter().any(|p| p == prefix),
            };
            if !applies {
                continue;
            }
            if !mine {
                ignored.push(ForeignMarker {
                    author: comment.author.clone(),
                    url: comment.url.clone(),
                });
                continue;
            }
            match marker {
                Marker::Point {
                    n,
                    state,
                    commit,
                    summary,
                    ..
                } => {
                    let marked = Marked {
                        state: PointState::marked(*state),
                        commit: commit.clone(),
                        summary: summary.clone(),
                        set_by: Some(comment.url.clone()),
                        set_at: Some(comment.created_at),
                    };
                    match split.iter_mut().find(|(k, _)| k == n) {
                        Some((_, point)) => *point = marked,
                        None => split.push((*n, marked)),
                    }
                }
                Marker::Legacy { state, .. } => {
                    let marked = Marked {
                        state: PointState::legacy(*state),
                        commit: None,
                        summary: None,
                        set_by: Some(comment.url.clone()),
                        set_at: Some(comment.created_at),
                    };
                    whole = marked.clone();
                    for (_, point) in &mut split {
                        *point = marked.clone();
                    }
                }
            }
        }
    }
    let reapproval = later_approval(review, reviews);
    let finish = |n: Option<u32>, mut marked: Marked| {
        // Approvals and markers apply in time order: an approval answers
        // only a state set before it (review on #97).
        if let Some((approval, approved_at)) = reapproval
            && matches!(marked.state, PointState::Open | PointState::NotYetSplit)
            && marked.set_at.is_none_or(|set_at| set_at < approved_at)
        {
            marked = Marked {
                state: PointState::Answered,
                commit: None,
                summary: None,
                set_by: Some(approval.url.clone()),
                set_at: Some(approved_at),
            };
        }
        Point {
            reference: PointRef::Review {
                review: review.id,
                n,
            },
            author: review.author.clone(),
            url: Some(review.url.clone()),
            state: marked.state,
            commit: marked.commit,
            summary: marked.summary,
            set_by: marked.set_by,
            ignored_markers: ignored.clone(),
        }
    };
    if split.is_empty() {
        return vec![finish(None, whole)];
    }
    split.sort_by_key(|(n, _)| *n);
    split
        .into_iter()
        .map(|(n, marked)| finish(Some(n), marked))
        .collect()
}

/// The latest approval by `review`'s author submitted after it, with when,
/// if any: the latest, so a marker is weighed against the newest approval.
fn later_approval<'a>(
    review: &ReviewRecord,
    reviews: &'a [ReviewRecord],
) -> Option<(&'a ReviewRecord, Timestamp)> {
    let (Some(author), Some(at)) = (&review.author, review.submitted_at) else {
        return None;
    };
    reviews
        .iter()
        .filter(|other| {
            other.verdict == ReviewVerdict::Approved
                && other
                    .author
                    .as_deref()
                    .is_some_and(|a| same_login(a, author))
        })
        .filter_map(|other| other.submitted_at.map(|t| (other, t)))
        .filter(|(_, t)| *t > at)
        .max_by_key(|(_, t)| *t)
}
