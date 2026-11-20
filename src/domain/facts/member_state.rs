//! What GitHub said of a Landing Group member den didn't find among the open
//! Pull Requests (#37). The survey reads open Pull Requests only, so a member
//! missing from a Repo whose open Pull Requests were all read is merged or
//! closed, and den can't tell which. Mark for release needs to know: a member
//! closed without merging is no evidence that anything landed. So den reads
//! those members' states by number after the survey, read-only, and records
//! them here with their own coverage (`SourceKind::MemberStates`).

use super::linear_issue::PullRequestRef;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// One member's state, as GitHub reported it when den asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberPullRequestState {
    pub pr: PullRequestRef,
    pub state: PullRequestState,
}

/// A member den asked GitHub about and got no state for (#37), and when it
/// last asked. The read is bounded, so den asks for members it never asked
/// about first, then the ones unanswered longest: a member that never
/// resolves (an attachment naming a Pull Request that doesn't exist) can't
/// keep the others from being read. Kept only while den still wants that
/// member's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberStateAttempt {
    pub pr: PullRequestRef,
    pub attempted_at: Timestamp,
}

/// A Pull Request's state on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullRequestState {
    /// Open again: it was missing from the open Pull Requests den read, but
    /// GitHub reported it open when den asked after the survey.
    Open,
    /// Merged, at this time when GitHub said.
    Merged { merged_at: Option<Timestamp> },
    /// Closed without merging.
    Closed,
}

impl PullRequestState {
    /// From GitHub's `PullRequest.state` (`OPEN`, `MERGED`, `CLOSED`); `None`
    /// for a state den doesn't know, which den never treats as any of these.
    pub fn from_github(state: &str, merged_at: Option<Timestamp>) -> Option<Self> {
        match state {
            "OPEN" => Some(Self::Open),
            "MERGED" => Some(Self::Merged { merged_at }),
            "CLOSED" => Some(Self::Closed),
            _ => None,
        }
    }
}
