//! Facts per Work Item kind: what the GitHub source observed, nothing decided.
//! One file per kind, owned by the ticket that adds that kind's rules
//! (see `docs/architecture.md`).

pub mod branch;
pub mod feedback;
pub mod issue;
pub mod linear_issue;
pub mod member_state;
pub mod pull_request;
pub mod run;

pub use branch::{Branch, ComparisonBranch};
pub use feedback::{LegacyState, MarkedState, Marker, MarkerComment, ReviewRecord, ThreadRecord};
pub use issue::{Issue, IssueLink, LinkKind, LinkedPrState};
pub use linear_issue::{
    Attachment, Attachments, AttachmentsRead, LinearCycle, LinearIssue, LinearPriority,
    LinearState, LinearStateKind, LinearTeam, LinearUser, PullRequestRef,
};
pub use member_state::{MemberPullRequestState, MemberStateAttempt, PullRequestState};
pub use pull_request::{
    ActorKind, Check, CheckOutcome, Listed, MergeState, Mergeability, PrDetails, PullRequest,
    Review, ReviewDecision, ReviewVerdict, Reviewer,
};
pub use run::{Run, RunConclusion, RunStatus};
