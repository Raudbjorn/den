//! Facts per Work Item kind: what the GitHub source observed, nothing decided.
//! One file per kind, owned by the ticket that adds that kind's rules
//! (see `docs/architecture.md`).

pub mod branch;
pub mod issue;
pub mod linear_issue;
pub mod pull_request;
pub mod run;

pub use branch::{Branch, ComparisonBranch};
pub use issue::{Issue, IssueLink, LinkKind, LinkedPrState};
pub use linear_issue::{
    LinearCycle, LinearIssue, LinearPriority, LinearState, LinearStateKind, LinearTeam, LinearUser,
};
pub use pull_request::{
    ActorKind, Check, CheckOutcome, Listed, MergeState, Mergeability, PrDetails, PullRequest,
    Review, ReviewDecision, ReviewVerdict, Reviewer,
};
pub use run::{Run, RunConclusion, RunStatus};
