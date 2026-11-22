//! Pull Request builder: one setter per fact the PR rules read.

use super::{repo_id, ts};
use den::domain::{
    TeamRef,
    facts::{
        ActorKind, Check, CheckOutcome, MarkerComment, MergeState, Mergeability, PrDetails,
        PullRequest, Review, ReviewDecision, ReviewRecord, ReviewVerdict, Reviewer, Screenshots,
        ThreadRecord, feedback::parse_markers,
    },
};

/// When a built PR was last updated, unless set.
const DEFAULT_UPDATED: &str = "2026-11-18T12:00:00Z";

/// A ready, non-draft PR in `repo` ("owner/name"), authored by "someone":
/// mergeable (and GitHub's merge state clean), review required by GitHub but
/// none given, no checks, no review requests, not queued, details read (at
/// the same head). No rule holds for it (it is not approved).
pub fn pr(repo: &str, number: u64) -> PrBuilder {
    let repo = repo_id(repo);
    let head_commit = format!("{number:040x}");
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
            details: Some(PrDetails {
                head_commit: Some(head_commit.clone()),
                ..PrDetails::default()
            }),
            head_commit,
            created_at: ts(DEFAULT_UPDATED),
            updated_at: ts(DEFAULT_UPDATED),
            mergeability: Mergeability::Mergeable,
            merge_state: MergeState::Clean,
            review_decision: ReviewDecision::ReviewRequired,
            in_merge_queue: false,
            screenshots: Screenshots::NotRead,
        },
    }
}

#[derive(Clone)]
pub struct PrBuilder {
    pr: PullRequest,
}

impl PrBuilder {
    /// The head branch lives in another Repo ("owner/name"): a cross-Repo PR.
    pub fn head_from(mut self, repo: &str) -> Self {
        self.pr.head_repo = Some(repo_id(repo));
        self
    }

    /// The head branch's name (default `feature-<n>`).
    pub fn head_ref(mut self, branch: &str) -> Self {
        branch.clone_into(&mut self.pr.head_ref);
        self
    }

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

    /// GitHub's own merge state (`mergeStateStatus`).
    pub fn merge_state(mut self, state: MergeState) -> Self {
        self.pr.merge_state = state;
        self
    }

    pub fn decision(mut self, decision: ReviewDecision) -> Self {
        self.pr.review_decision = decision;
        self
    }

    /// GitHub reports no review decision: the base branch requires no review.
    pub fn no_review_required(self) -> Self {
        self.decision(ReviewDecision::NotRequired)
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

    /// Some review requests could not be read.
    pub fn review_requests_truncated(mut self) -> Self {
        self.details().review_requests.complete = false;
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
        self.pr.review_decision = ReviewDecision::NotRequired;
        let commit = self.pr.head_commit.clone();
        self.latest_review(login, ActorKind::User, verdict, &commit)
    }

    /// The head commit (default: the PR number as 40 hex digits), as both
    /// the PR list and the details query read it.
    pub fn head(mut self, commit: &str) -> Self {
        commit.clone_into(&mut self.pr.head_commit);
        if let Some(details) = &mut self.pr.details {
            details.head_commit = Some(commit.to_owned());
        }
        self
    }

    /// Someone pushed `commit` between the PR list and the details query:
    /// the details (reviews included) were read at `commit`.
    pub fn details_read_at(mut self, commit: &str) -> Self {
        self.details().head_commit = Some(commit.to_owned());
        self
    }

    /// A user's latest review, given on the current head. GitHub's review
    /// decision is left as it is.
    pub fn review_at_head(self, login: &str, verdict: ReviewVerdict) -> Self {
        let commit = self.pr.head_commit.clone();
        self.latest_review(login, ActorKind::User, verdict, &commit)
    }

    /// A user's latest review, given on `commit` (an older head, say).
    pub fn review_on(self, login: &str, verdict: ReviewVerdict, commit: &str) -> Self {
        self.latest_review(login, ActorKind::User, verdict, commit)
    }

    /// A bot's latest review, given on the current head.
    pub fn bot_review_at_head(self, login: &str, verdict: ReviewVerdict) -> Self {
        let commit = self.pr.head_commit.clone();
        self.latest_review(login, ActorKind::Bot, verdict, &commit)
    }

    /// A latest review as GitHub lists it: it becomes `login`'s latest review
    /// of any kind (`latestReviews`), and, unless it only comments, also the
    /// verdict that stands (`latestOpinionatedReviews`).
    pub fn latest_review(
        mut self,
        login: &str,
        kind: ActorKind,
        verdict: ReviewVerdict,
        commit: &str,
    ) -> Self {
        let review = Review {
            author: Some(login.to_owned()),
            author_kind: kind,
            verdict,
            commit: Some(commit.to_owned()),
        };
        let details = self.details();
        let by = |r: &Review| r.author.as_deref() == Some(login);
        details
            .latest_reviews_with_comments
            .items
            .retain(|r| !by(r));
        details
            .latest_reviews_with_comments
            .items
            .push(review.clone());
        if verdict != ReviewVerdict::Commented {
            details.latest_reviews.items.retain(|r| !by(r));
            details.latest_reviews.items.push(review);
        }
        self
    }

    /// GitHub listed more reviews than den read.
    pub fn reviews_truncated(mut self) -> Self {
        let details = self.details();
        details.latest_reviews.complete = false;
        details.latest_reviews_with_comments.complete = false;
        self
    }

    /// GitHub listed more latest reviews of any kind (`latestReviews`) than
    /// den read; the standing verdicts were read in full.
    pub fn latest_reviews_truncated(mut self) -> Self {
        self.details().latest_reviews_with_comments.complete = false;
        self
    }

    /// A submitted review for Review Points (#75): `id` (GitHub's number),
    /// by the user `login`, at `at`; `body` is whether it says more than a
    /// stock approval. GitHub's review decision is left as it is.
    pub fn review_body(
        self,
        id: u64,
        login: &str,
        verdict: ReviewVerdict,
        at: &str,
        body: bool,
    ) -> Self {
        self.review_record(id, Some(login), ActorKind::User, verdict, at, body)
    }

    /// A submitted review for Review Points with every field given.
    pub fn review_record(
        mut self,
        id: u64,
        login: Option<&str>,
        kind: ActorKind,
        verdict: ReviewVerdict,
        at: &str,
        body: bool,
    ) -> Self {
        let url = format!("{}#pullrequestreview-{id}", self.pr.url);
        self.details().reviews.items.push(ReviewRecord {
            id,
            author: login.map(str::to_owned),
            author_kind: kind,
            verdict,
            submitted_at: Some(ts(at)),
            url,
            has_body: body,
        });
        self
    }

    /// A review thread `id` started by `login`, resolved or not. Also counts
    /// into the unresolved threads, as the decoder does from the same list.
    pub fn thread(mut self, id: &str, login: &str, resolved: bool) -> Self {
        let url = format!("{}#discussion_{id}", self.pr.url);
        let details = self.details();
        details.threads.items.push(ThreadRecord {
            id: id.to_owned(),
            url: Some(url),
            author: Some(login.to_owned()),
            resolved,
        });
        if !resolved {
            details.unresolved_threads += 1;
        }
        self
    }

    /// A conversation comment by `login` at `at` with `body`, kept as the
    /// decoder keeps it: its markers only, with summaries when `login` is
    /// the viewer (`ME`). Comments without markers are not kept.
    pub fn comment(mut self, comment: u64, login: &str, at: &str, body: &str) -> Self {
        let markers = parse_markers(body, login == super::ME);
        let url = format!("{}#issuecomment-{comment}", self.pr.url);
        if !markers.is_empty() {
            self.details().marker_comments.items.push(MarkerComment {
                author: Some(login.to_owned()),
                created_at: ts(at),
                url,
                markers,
            });
        }
        self
    }

    /// GitHub had more reviews (for Review Points) than den read.
    pub fn point_reviews_truncated(mut self) -> Self {
        self.details().reviews.complete = false;
        self
    }

    /// GitHub had more conversation comments than den read.
    pub fn comments_truncated(mut self) -> Self {
        self.details().marker_comments.complete = false;
        self
    }

    fn details(&mut self) -> &mut PrDetails {
        let head = self.pr.head_commit.clone();
        self.pr.details.get_or_insert_with(|| PrDetails {
            head_commit: Some(head),
            ..PrDetails::default()
        })
    }

    /// The body's Screenshots state (#79); not read by default.
    pub fn screenshots(mut self, state: Screenshots) -> Self {
        self.pr.screenshots = state;
        self
    }

    pub fn build(self) -> PullRequest {
        self.pr
    }
}
