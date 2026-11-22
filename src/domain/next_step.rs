//! Next Step and Waiting On (CONTEXT.md, "Kinds of Next Step" and "Waiting On").

use serde::{Deserialize, Serialize};

/// The single action a Work Item needs next. Each Work Item has at most one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NextStep {
    /// A draft Pull Request needs completing.
    FinishDraft,
    /// A Pull Request cannot merge because of conflicts.
    ResolveConflicts,
    /// Checks fail on a Pull Request, or a workflow's latest Run on a default branch failed.
    FixCi,
    /// A Pull Request has requested changes or unresolved review threads.
    AddressReview,
    /// The viewer's own Pull Request, in a Repo with `screenshots = true`,
    /// has neither a Screenshots section with an image nor a "No visual
    /// change" line (#79).
    AddScreenshots,
    /// A Pull Request is waiting for a requested review.
    Review,
    /// A Pull Request is approved, passing and mergeable.
    Merge,
    /// An Issue has neither labels nor an assignee.
    Triage,
    /// An Issue is ready to be worked on.
    Implement,
    /// A Branch has unmerged commits and no Pull Request.
    OpenPr,
    /// A Stale Branch needs a decision.
    PruneOrRevive,
    /// An unassigned, not-yet-started Linear Issue can be picked up (#33).
    Start,
    /// A Pull Request needs reviewers asked, or asked again (#36).
    RequestReview,
    /// A Landing Group is ready to land (#37).
    Land,
    /// A landed Linear Issue can move to the release state (#37).
    MarkForRelease,
}

impl NextStep {
    pub const ALL: [Self; 15] = [
        Self::FinishDraft,
        Self::ResolveConflicts,
        Self::FixCi,
        Self::AddressReview,
        Self::AddScreenshots,
        Self::Review,
        Self::Merge,
        Self::Triage,
        Self::Implement,
        Self::OpenPr,
        Self::PruneOrRevive,
        Self::Start,
        Self::RequestReview,
        Self::Land,
        Self::MarkForRelease,
    ];

    /// The glossary name, as shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::FinishDraft => "Finish draft",
            Self::ResolveConflicts => "Resolve conflicts",
            Self::FixCi => "Fix CI",
            Self::AddressReview => "Address review",
            Self::AddScreenshots => "Add screenshots",
            Self::Review => "Review",
            Self::Merge => "Merge",
            Self::Triage => "Triage",
            Self::Implement => "Implement",
            Self::OpenPr => "Open PR",
            Self::PruneOrRevive => "Prune or revive",
            Self::Start => "Start",
            Self::RequestReview => "Request review",
            Self::Land => "Land",
            Self::MarkForRelease => "Mark for release",
        }
    }

    /// The Next Step whose glossary name is `label`, spelled exactly as
    /// [`NextStep::label`] (and `CONTEXT.md`) spells it.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.label() == label)
    }

    /// Whether a Repo may stop tracking this kind (`skip`, spec 0003). Start,
    /// Land and Mark for release belong to Linear Issues, not to a Repo.
    pub fn skippable(self) -> bool {
        !matches!(self, Self::Start | Self::Land | Self::MarkForRelease)
    }
}

/// Who a Next Step is owed by. "Unattributed" is a separate marker on the
/// Decision, not a fourth value (spec, Further Notes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WaitingOn {
    Me,
    Nobody,
    SomeoneElse,
}

impl WaitingOn {
    pub fn label(self) -> &'static str {
        match self {
            Self::Me => "you",
            Self::Nobody => "nobody",
            Self::SomeoneElse => "someone else",
        }
    }
}
