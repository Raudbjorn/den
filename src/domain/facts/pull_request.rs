//! Pull Request facts: what GitHub said about an open PR, nothing decided.
//!
//! The cheap facts (draft, mergeability, merge state, review decision, merge
//! queue) come with
//! the PR list; checks, review threads, review requests and latest reviews come
//! from a follow-up query and live in [`PrDetails`], which is `None` when that
//! query could not be read (coverage then says why).

use crate::domain::{
    ids::{RepoId, WorkItemId},
    observation::TeamRef,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub url: String,
    /// `None` when GitHub has no author (a deleted account).
    pub author: Option<String>,
    pub is_draft: bool,
    pub base_ref: String,
    pub head_ref: String,
    pub head_commit: String,
    /// The Repo the head branch lives in; differs from `repo` for cross-Repo PRs.
    /// `None` when that Repo was deleted.
    pub head_repo: Option<RepoId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub mergeability: Mergeability,
    /// GitHub's own verdict on whether the PR can merge now.
    pub merge_state: MergeState,
    /// GitHub's own review decision.
    pub review_decision: ReviewDecision,
    pub in_merge_queue: bool,
    /// Checks and reviews; `None` when den could not read them.
    pub details: Option<PrDetails>,
}

impl PullRequest {
    pub fn id(&self) -> WorkItemId {
        WorkItemId::PullRequest {
            repo: self.repo.clone(),
            number: self.number,
        }
    }
}

/// Whether the PR can merge into its base, as GitHub computed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mergeability {
    Mergeable,
    Conflicting,
    /// GitHub has not computed it yet (it does so lazily, after pushes).
    Unknown,
}

/// GitHub's `mergeStateStatus`: whether the PR can merge right now, judged
/// against every rule of the base branch, including those den does not read
/// (a required check that never reported, a second required approval, a
/// required code-owner review).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeState {
    /// Mergeable, every commit status passing.
    Clean,
    /// Mergeable, statuses passing, pre-receive hooks to run.
    HasHooks,
    /// Mergeable, but a commit status is not passing.
    Unstable,
    /// A base-branch rule blocks merging.
    Blocked,
    /// The head branch must be brought up to date with the base first.
    Behind,
    /// A merge commit cannot be cleanly created.
    Dirty,
    Draft,
    /// GitHub has not computed it yet, or den does not recognise the value.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    /// GitHub reported no decision: the base branch requires no review (GitHub
    /// then reports none even if someone reviewed).
    NotRequired,
    /// den could not tell: the decision came back null in a response that
    /// carried errors (which can null a field), or in a form den does not know.
    Unknown,
}

/// The facts read by the follow-up query. The default is "read completely,
/// nothing found", at an unknown head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrDetails {
    /// The PR's head commit as this query read it. A push between the PR list
    /// and this query makes it differ from [`PullRequest::head_commit`]; the
    /// reviews' commits are only comparable to this one. `None`: not known.
    pub head_commit: Option<String>,
    /// Checks on the head commit (check runs and commit statuses).
    pub checks: Listed<Check>,
    /// Review threads not yet resolved, among those read.
    pub unresolved_threads: u32,
    /// Every review thread was read (so `unresolved_threads` is exact).
    pub threads_complete: bool,
    /// Pending review requests.
    pub review_requests: Listed<Reviewer>,
    /// Each reviewer's latest approving or change-requesting review
    /// (GitHub's `latestOpinionatedReviews`): the verdict that stands.
    pub latest_reviews: Listed<Review>,
    /// Each reviewer's latest review of any kind, comments included, never a
    /// pending one (GitHub's `latestReviews`): who looked last, and at which
    /// commit. Read for the Linear Workflow review rules (#36); milestone 1's
    /// rules don't use it.
    pub latest_reviews_with_comments: Listed<Review>,
}

impl Default for PrDetails {
    fn default() -> Self {
        Self {
            head_commit: None,
            checks: Listed::default(),
            unresolved_threads: 0,
            threads_complete: true,
            review_requests: Listed::default(),
            latest_reviews: Listed::default(),
            latest_reviews_with_comments: Listed::default(),
        }
    }
}

/// A list as far as den read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed<T> {
    pub items: Vec<T>,
    /// False when GitHub had more than den read, or some entries came back
    /// unreadable.
    pub complete: bool,
}

impl<T> Default for Listed<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            complete: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// `workflow / job` for Actions check runs, the context for statuses.
    pub name: String,
    /// Required by the base branch's protection or rulesets for this PR.
    pub required: bool,
    pub outcome: CheckOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckOutcome {
    Passing,
    Failing,
    /// Queued, running or expected.
    Pending,
    /// A state den does not know, as GitHub spelled it.
    Unrecognised(String),
}

/// Someone asked to review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reviewer {
    User(String),
    Team(TeamRef),
}

impl Reviewer {
    /// How den names the reviewer: the login, or "team org/slug".
    pub fn label(&self) -> String {
        match self {
            Self::User(login) => login.clone(),
            Self::Team(team) => format!("team {}/{}", team.org, team.slug),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// `None` when GitHub has no author (a deleted account).
    pub author: Option<String>,
    /// What kind of account the author is.
    pub author_kind: ActorKind,
    pub verdict: ReviewVerdict,
    /// The commit the review was given on; `None` when GitHub no longer has it
    /// (for example after a force-push removed it).
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewVerdict {
    Approved,
    ChangesRequested,
    /// A review that only comments.
    Commented,
    /// Dismissed or another state that neither approves nor blocks.
    Other,
}

/// GitHub's kind of account (`Actor.__typename`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActorKind {
    User,
    Bot,
    /// A mannequin, an organization, or a kind den doesn't know, or no author.
    Other,
}
