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

use super::facts::LinearIssue;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// What den read from Linear in one survey.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearPart {
    /// The Linear viewer of each Linear Workflow whose viewer den could read.
    pub viewers: Vec<LinearViewer>,
    /// The Linear Issues read, of every Owner with a Linear Workflow.
    pub issues: Vec<LinearIssue>,
    /// When the Linear survey that read this part started; `None` before any
    /// Linear survey answered. Linear is surveyed apart from GitHub, so its
    /// data has its own age: the cache labels Linear data it keeps with it
    /// (#34), never with the GitHub survey's time.
    pub observed_at: Option<Timestamp>,
}

impl LinearPart {
    /// Each Owner this part holds Linear data for, with the workspace a fact
    /// of it was read in: its Linear viewers' and its Linear Issues'. Linear
    /// data is kept only for the workspace it was read in, so both the cache
    /// load (`CachedObservation::restricted_to_linear`) and a Linear refresh
    /// (`cache::merge_linear`) judge "read elsewhere" from this one list; a
    /// new Linear fact list joins it here.
    pub fn workspaces(&self) -> impl Iterator<Item = (&str, &str)> {
        let viewers = self
            .viewers
            .iter()
            .map(|v| (v.owner.as_str(), v.workspace.as_str()));
        let issues = self
            .issues
            .iter()
            .map(|i| (i.owner.as_str(), i.id.workspace.as_str()));
        viewers.chain(issues)
    }
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
