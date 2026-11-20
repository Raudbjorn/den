//! den's domain model, in the glossary's terms (CONTEXT.md).

pub mod coverage;
pub mod facts;
pub mod ids;
pub mod next_step;
pub mod observation;

pub use coverage::{Coverage, CoverageEntry, Incompleteness, Source, SourceKind, SourceState};
pub use ids::{RepoId, WorkItemId, WorkItemKind, WorkflowId};
pub use next_step::{NextStep, WaitingOn};
pub use observation::{Observation, Repo, TeamRef, Viewer};
