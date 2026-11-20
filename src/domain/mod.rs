//! den's domain model, in the glossary's terms (CONTEXT.md).

pub mod coverage;
pub mod facts;
pub mod ids;
pub mod linear;
pub mod next_step;
pub mod observation;
pub mod policy;
pub mod upstream;

pub use coverage::{
    Coverage, CoverageEntry, Incompleteness, LinearSourceKind, Source, SourceKind, SourceState,
};
pub use ids::{LinearIssueId, RepoId, WorkItemId, WorkItemKind, WorkflowId};
pub use linear::{LinearPart, LinearTeamStates, LinearViewer};
pub use next_step::{NextStep, WaitingOn};
pub use observation::{Observation, Repo, TeamRef, Viewer, Viewers};
pub use policy::{LinearWorkflowPolicy, Policies, ReviewPolicy, StartPolicy};
pub use upstream::{Comparison, UpstreamTargets, comparison_for};
