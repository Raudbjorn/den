//! The Linear part of an Observation (spec 0002, "Linear source").
//!
//! Milestone-2 groundwork (#31): the slot exists and is cached with the rest of
//! the Observation, but nothing fills it yet. Later tickets add, per Owner with a
//! Linear Workflow:
//!
//! - #32: the Linear viewer of the workflow's workspace and the Linear Issues
//!   assigned to it (identity: workspace + permanent ID, never the identifier);
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

use serde::{Deserialize, Serialize};

/// What den read from Linear in one survey. Empty until #32.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearPart {}
