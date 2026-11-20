//! The Linear part of an Observation (spec 0002, "Linear source").
//!
//! Per Owner with a Linear Workflow:
//!
//! - #32: the Linear viewer of the workflow's workspace and the active Linear
//!   Issues assigned to it (identity: workspace + permanent ID, never the
//!   identifier);
//! - #33: the unassigned Linear Issues of the start teams, and each team's
//!   workflow states;
//! - #35: each Linear Issue's attachments, reduced to PR identities.
//!
//! Its coverage lives in the Observation's one [`Coverage`](super::Coverage),
//! under [`SourceKind::Linear`](super::SourceKind::Linear), so that the header,
//! the sidebar and "Nothing needs you" judge Linear sources as they judge
//! GitHub's. An empty part with no Linear sources recorded is what an
//! Observation of Owners without a Linear Workflow holds; it never makes the
//! survey read as incomplete.

use super::facts::{LinearIssue, LinearState, LinearTeam};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// What den read from Linear in one survey.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearPart {
    /// The Linear viewer of each Linear Workflow whose viewer den could read.
    pub viewers: Vec<LinearViewer>,
    /// The Linear Issues read, of every Owner with a Linear Workflow: those
    /// assigned to the Linear viewer and the start teams' unassigned ones.
    pub issues: Vec<LinearIssue>,
    /// Each team's workflow states, per Linear Workflow that names a start
    /// team or a state (#33).
    #[serde(default)]
    pub team_states: Vec<LinearTeamStates>,
    /// When the Linear survey that read this part started; `None` before any
    /// Linear survey answered. Linear is surveyed apart from GitHub, so its
    /// data has its own age: the cache labels Linear data it keeps with it
    /// (#34), never with the GitHub survey's time.
    pub observed_at: Option<Timestamp>,
}

/// One Linear team and its workflow states, as one Linear Workflow read them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearTeamStates {
    /// The Owner whose Linear Workflow read it.
    pub owner: String,
    /// The workspace's URL key, as the config names it.
    pub workspace: String,
    pub team: LinearTeam,
    /// The team's display name.
    pub name: String,
    /// The team's workflow states, in the order Linear listed them.
    pub states: Vec<LinearState>,
}

/// Who the Linear API key belongs to, in one workspace: "me" for Linear
/// Issues of that workspace. Never matched to a GitHub login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearViewer {
    /// The Owner whose Linear Workflow read it.
    pub owner: String,
    /// The workspace's URL key, as the config names it.
    pub workspace: String,
    /// Linear's permanent user ID.
    pub id: String,
    pub name: String,
}

impl LinearViewer {
    /// Whether the Linear user `id` is this viewer.
    pub fn is(&self, id: &str) -> bool {
        self.id == id
    }
}

impl LinearPart {
    /// The Linear viewer of `workspace`, if den read it.
    pub fn viewer(&self, workspace: &str) -> Option<&LinearViewer> {
        self.viewers
            .iter()
            .find(|v| v.workspace.eq_ignore_ascii_case(workspace))
    }
}
