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
}

impl NextStep {
    pub const ALL: [Self; 10] = [
        Self::FinishDraft,
        Self::ResolveConflicts,
        Self::FixCi,
        Self::AddressReview,
        Self::Review,
        Self::Merge,
        Self::Triage,
        Self::Implement,
        Self::OpenPr,
        Self::PruneOrRevive,
    ];

    /// The glossary name, as shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::FinishDraft => "Finish draft",
            Self::ResolveConflicts => "Resolve conflicts",
            Self::FixCi => "Fix CI",
            Self::AddressReview => "Address review",
            Self::Review => "Review",
            Self::Merge => "Merge",
            Self::Triage => "Triage",
            Self::Implement => "Implement",
            Self::OpenPr => "Open PR",
            Self::PruneOrRevive => "Prune or revive",
        }
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
