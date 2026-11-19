//! Pull Request builder: one setter per fact the PR rules read.

use super::{repo_id, ts};
use den::domain::{
    TeamRef,
    facts::{
        Check, CheckOutcome, Mergeability, PrDetails, PullRequest, Review, ReviewDecision,
        ReviewVerdict, Reviewer,
    },
};

/// When a built PR was last updated, unless set.
const DEFAULT_UPDATED: &str = "2026-11-18T12:00:00Z";

/// A ready, non-draft PR in `repo` ("owner/name"), authored by "someone":
/// mergeable, review required by GitHub but none given, no checks, no review
/// requests, not queued, details read. No rule holds for it (it is not approved).
pub fn pr(repo: &str, number: u64) -> PrBuilder {
    let repo = repo_id(repo);
    PrBuilder {
        pr: PullRequest {
            url: format!("https://github.com/{repo}/pull/{number}"),
            head_repo: Some(repo.clone()),
            repo,
            number,
            title: format!("PR {number}"),
            author: Some("someone".to_owned()),
            is_draft: false,
            base_ref: "main".to_owned(),
            head_ref: format!("feature-{number}"),
            head_commit: format!("{number:040x}"),
            created_at: ts(DEFAULT_UPDATED),
            updated_at: ts(DEFAULT_UPDATED),
            mergeability: Mergeability::Mergeable,
            review_decision: Some(ReviewDecision::ReviewRequired),
            in_merge_queue: false,
            details: Some(PrDetails::default()),
        },
    }
}

pub struct PrBuilder {
    pr: PullRequest,
}

impl PrBuilder {
    pub fn draft(mut self) -> Self {
        self.pr.is_draft = true;
        self
    }

    pub fn author(mut self, login: &str) -> Self {
        self.pr.author = Some(login.to_owned());
        self
    }

    /// GitHub has no author (a deleted account).
    pub fn no_author(mut self) -> Self {
        self.pr.author = None;
        self
    }

    pub fn title(mut self, title: &str) -> Self {
        self.pr.title = title.to_owned();
        self
    }

    pub fn updated(mut self, at: &str) -> Self {
        self.pr.updated_at = ts(at);
        self
    }

    pub fn conflicting(mut self) -> Self {
        self.pr.mergeability = Mergeability::Conflicting;
        self
    }

    /// GitHub has not computed mergeability yet.
    pub fn mergeability_unknown(mut self) -> Self {
        self.pr.mergeability = Mergeability::Unknown;
        self
    }

    pub fn decision(mut self, decision: ReviewDecision) -> Self {
        self.pr.review_decision = Some(decision);
        self
    }

    /// GitHub reports no review decision: the base branch requires no review.
    pub fn no_review_required(mut self) -> Self {
        self.pr.review_decision = None;
        self
    }

    /// GitHub's review decision is APPROVED.
    pub fn approved(self) -> Self {
        self.decision(ReviewDecision::Approved)
    }

    /// GitHub's review decision is CHANGES_REQUESTED.
    pub fn changes_requested(self) -> Self {
        self.decision(ReviewDecision::ChangesRequested)
    }

    pub fn queued(mut self) -> Self {
        self.pr.in_merge_queue = true;
        self
    }

    /// The details query was lost: checks and reviews are unknown.
    pub fn details_unread(mut self) -> Self {
        self.pr.details = None;
        self
    }

    pub fn check(mut self, name: &str, required: bool, outcome: CheckOutcome) -> Self {
        self.details().checks.items.push(Check {
            name: name.to_owned(),
            required,
            outcome,
        });
        self
    }

    pub fn passing(self, name: &str) -> Self {
        self.check(name, true, CheckOutcome::Passing)
    }

    pub fn failing_required(self, name: &str) -> Self {
        self.check(name, true, CheckOutcome::Failing)
    }

    pub fn failing_advisory(self, name: &str) -> Self {
        self.check(name, false, CheckOutcome::Failing)
    }

    pub fn pending(self, name: &str) -> Self {
        self.check(name, true, CheckOutcome::Pending)
    }

    /// GitHub listed more checks than den read.
    pub fn checks_truncated(mut self) -> Self {
        self.details().checks.complete = false;
        self
    }

    /// GitHub listed more review threads than den read.
    pub fn threads_truncated(mut self) -> Self {
        self.details().threads_complete = false;
        self
    }

    pub fn unresolved_threads(mut self, count: u32) -> Self {
        self.details().unresolved_threads = count;
        self
    }

    pub fn review_requested_from(mut self, login: &str) -> Self {
        self.details()
            .review_requests
            .items
            .push(Reviewer::User(login.to_owned()));
        self
    }

    pub fn review_requested_from_team(mut self, org: &str, slug: &str) -> Self {
        self.details()
            .review_requests
            .items
            .push(Reviewer::Team(TeamRef {
                org: org.to_owned(),
                slug: slug.to_owned(),
            }));
        self
    }

    /// A latest review by `login`, on a Repo that requires no review (so
    /// GitHub reports no decision and den reads the reviews itself).
    pub fn reviewed(mut self, login: &str, verdict: ReviewVerdict) -> Self {
        self.pr.review_decision = None;
        self.details().latest_reviews.items.push(Review {
            author: Some(login.to_owned()),
            verdict,
        });
        self
    }

    fn details(&mut self) -> &mut PrDetails {
        self.pr.details.get_or_insert_with(PrDetails::default)
    }

    pub fn build(self) -> PullRequest {
        self.pr
    }
}
