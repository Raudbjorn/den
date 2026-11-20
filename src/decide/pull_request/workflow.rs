//! The Linear Workflow's review rules for one PR (spec 0002, "Next Step rules:
//! PRs of an Owner with a Linear Workflow"). Only PRs of an Owner with a
//! Linear Workflow reach this module; every other PR keeps milestone 1's
//! rules in the parent module.
//!
//! Reviews are judged against the head read in the same query as they were
//! (`PrDetails::head_commit`). If that differs from the PR list's head (a
//! push landed between the two queries), den can't tell which head they
//! were on relative to the PR's facts, and treats the reviews as unread.
//!
//! Two of GitHub's review lists are read, per reviewer:
//! - the **standing** review (`latestOpinionatedReviews`): the approval or
//!   change request that still stands, and the commit it was given on;
//! - the **latest** review of any kind (`latestReviews`, comments included):
//!   what the reviewer said last, and on which commit.
//!
//! Definitions (each condition is marked as workflow policy where it shows):
//! - **Fresh approval**, with exact-head approval on: a user's (never a bot's)
//!   standing approval given on the current head. With it off: GitHub's review
//!   decision, as in milestone 1. A comment after an approval at the same head
//!   doesn't withdraw it, as it doesn't on GitHub.
//! - **Approved**, with exact-head approval on: a fresh approval, and GitHub's
//!   own review decision not asking for more (not REVIEW_REQUIRED, not
//!   CHANGES_REQUESTED). The switch tightens GitHub's decision; it never
//!   replaces it, so a second required approval or a code owner's is still
//!   asked for. With it off: GitHub's review decision, as in milestone 1.
//! - **Awaiting re-review**: a user other than the author, not currently
//!   requested, not a fresh approver and whose latest review of any kind is
//!   not on the current head, whose standing or latest review requested
//!   changes on an older head, or whose latest review only commented on an
//!   older head when comment reviews need re-review. A review whose commit
//!   GitHub no longer has counts as on an older head. Someone who has since
//!   reviewed the current head has had the fresh look, even if their older
//!   change request still stands on GitHub.
//! - **Address review** holds on unresolved threads, or on changes requested
//!   at the current head (by anyone); changes requested on an older head fall
//!   through. Changes requested at the current head also stop Merge.
//! - **Suggested reviewers**: the configured reviewers plus those awaiting
//!   re-review, minus fresh approvers, pending requests and the author.

use super::{Approval, Finding, ReviewFindings, approval as github_approval, unresolved_threads};
use crate::domain::{
    ReviewPolicy,
    facts::{ActorKind, PrDetails, PullRequest, Review, ReviewDecision, ReviewVerdict, Reviewer},
    ids::same_login,
};

const EXACT_HEAD: &str = "workflow policy: approval must be at the current head";
const NO_BOTS: &str = "workflow policy: bots' approvals don't count";
const FRESH_LOOK: &str = "workflow policy: feedback given on an older head needs a fresh look";
const COMMENTS: &str = "workflow policy: comment reviews need re-review";
const AT_HEAD: &str = "workflow policy: only changes requested at the current head need addressing";
const DELETED_ACCOUNT: &str = "a deleted account";
const NOT_EVERY_REVIEW: &str = "not every review was read";
const HEAD_MOVED: &str = "the PR's head moved while den read its reviews";
const REVIEWS_NOT_READ: &str = "the reviews could not be read";

/// The workflow's review findings for one PR.
pub(super) fn assess(pr: &PullRequest, policy: &ReviewPolicy) -> ReviewFindings {
    let review = WorkflowReview::assess(pr, policy);
    let approval = review.approval();
    let approved = review.approved_text(&approval);
    ReviewFindings {
        address_review: review.address_review(),
        request_review: Some(review.request_review(&approval)),
        merge: format!("{approved}, nobody awaiting re-review ({FRESH_LOOK})"),
        approved,
        merge_blockers: review.merge_blockers(),
        notes: review.notes(),
        suggested_reviewers: Some(review.suggested),
        approval,
    }
}

/// What the workflow's review rules found for one PR.
struct WorkflowReview<'a> {
    pr: &'a PullRequest,
    exact_head: bool,
    read: ReviewsRead,
    suggested: Vec<Reviewer>,
}

/// What the PR's reviews say under the workflow; nothing when they were not
/// read.
#[derive(Default)]
struct ReviewsRead {
    fresh_approvers: Vec<String>,
    /// Approvals that don't count with exact-head approval on, each saying why
    /// ("alice approved an older head").
    discounted: Vec<String>,
    /// Some discounted approval came from a bot.
    bot_discounted: bool,
    awaiting: Vec<Awaiting>,
    /// Who requested changes at the current head.
    changes_at_head: Vec<String>,
    /// Who requested changes on an older head and isn't awaiting re-review
    /// (a bot, the author, or someone asked again).
    changes_earlier: Vec<String>,
}

/// A user whose feedback needs a fresh look.
struct Awaiting {
    login: String,
    commented_only: bool,
}

impl<'a> WorkflowReview<'a> {
    fn assess(pr: &'a PullRequest, policy: &ReviewPolicy) -> Self {
        let exact_head = policy.exact_head_approval;
        let requests = pr
            .details
            .as_ref()
            .map(|d| d.review_requests.items.as_slice());
        let Some(details) = pr.details.as_ref().filter(|d| same_head(pr, d)) else {
            return Self {
                pr,
                exact_head,
                read: ReviewsRead::default(),
                suggested: suggested(&policy.reviewers, &[], pr, requests, &[]),
            };
        };
        let reading = Reading {
            pr,
            details,
            exact_head,
        };
        let fresh_approvers = reading.fresh_approvers();
        let (discounted, bot_discounted) = reading.discounted(&fresh_approvers);
        let awaiting = reading.awaiting(&fresh_approvers, policy.comment_reviews_need_rereview);
        let changes_earlier = reading
            .changes_requested(false)
            .into_iter()
            .filter(|login| !awaiting.iter().any(|a| same_login(&a.login, login)))
            .collect();
        let waiting_logins: Vec<String> = awaiting.iter().map(|a| a.login.clone()).collect();
        Self {
            pr,
            exact_head,
            suggested: suggested(
                &policy.reviewers,
                &waiting_logins,
                pr,
                requests,
                &fresh_approvers,
            ),
            read: ReviewsRead {
                changes_at_head: reading.changes_requested(true),
                changes_earlier,
                fresh_approvers,
                discounted,
                bot_discounted,
                awaiting,
            },
        }
    }

    /// Why not every standing review counts as read, if so.
    fn standing_unread(&self) -> Option<&'static str> {
        match &self.pr.details {
            None => Some(REVIEWS_NOT_READ),
            Some(details) if !same_head(self.pr, details) => Some(HEAD_MOVED),
            Some(details) if !details.latest_reviews.complete => Some(NOT_EVERY_REVIEW),
            Some(_) => None,
        }
    }

    /// Why not every standing and every latest review counts as read, if so.
    fn reviews_unread(&self) -> Option<&'static str> {
        self.standing_unread().or_else(|| {
            self.pr
                .details
                .as_ref()
                .is_some_and(|d| !d.latest_reviews_with_comments.complete)
                .then_some(NOT_EVERY_REVIEW)
        })
    }

    /// Whether the PR is approved under the workflow: with exact-head approval
    /// on, a fresh approval that GitHub's own review decision doesn't
    /// contradict; GitHub's review decision as in milestone 1 with it off.
    fn approval(&self) -> Approval {
        if !self.exact_head {
            return github_approval(self.pr);
        }
        match (self.exact_head_approval(), github_decision(self.pr)) {
            (Approval::NotApproved(ours), Approval::NotApproved(github)) => {
                Approval::NotApproved(format!("{ours}; {github}"))
            }
            (Approval::NotApproved(why), _) | (_, Approval::NotApproved(why)) => {
                Approval::NotApproved(why)
            }
            (Approval::Unknown, _) | (_, Approval::Unknown) => Approval::Unknown,
            _ => Approval::Approved,
        }
    }

    /// Whether some user approved at the current head.
    fn exact_head_approval(&self) -> Approval {
        if !self.read.fresh_approvers.is_empty() {
            return Approval::Approved;
        }
        if self.standing_unread().is_some() {
            return Approval::Unknown;
        }
        let mut policies = vec![EXACT_HEAD];
        if self.read.bot_discounted {
            policies.push(NO_BOTS);
        }
        let why = if self.read.discounted.is_empty() {
            format!("no approval at the current head ({})", policies.join("; "))
        } else {
            format!(
                "no approval at the current head: {} ({})",
                self.read.discounted.join(", "),
                policies.join("; ")
            )
        };
        Approval::NotApproved(why)
    }

    /// "approved at the current head by alice (workflow policy: …)", or
    /// milestone 1's words when exact-head approval is off.
    fn approved_text(&self, approval: &Approval) -> String {
        if self.exact_head {
            return format!(
                "approved at the current head by {} ({EXACT_HEAD})",
                self.read.fresh_approvers.join(", ")
            );
        }
        match approval {
            Approval::NoReviewRequired => "no review required and no changes requested".to_owned(),
            _ => "approved".to_owned(),
        }
    }

    /// Address review: unresolved threads, or changes requested at the
    /// current head. Changes requested on an older head fall through.
    fn address_review(&self) -> Finding {
        let Some(details) = &self.pr.details else {
            return Finding::Unknown("review threads could not be read".to_owned());
        };
        let mut reasons: Vec<String> = self.changes_at_head_text().into_iter().collect();
        reasons.extend(unresolved_threads(details));
        if !reasons.is_empty() {
            return Finding::Holds(reasons.join(", "));
        }
        if !details.threads_complete {
            return Finding::Unknown("not every review thread was read".to_owned());
        }
        if let Some(why) = self.standing_unread() {
            return Finding::Unknown(why.to_owned());
        }
        Finding::No
    }

    /// Request review: no fresh approval, or someone awaiting re-review, and
    /// no review request pending.
    fn request_review(&self, approval: &Approval) -> Finding {
        let Some(details) = &self.pr.details else {
            return Finding::Unknown("review requests could not be read".to_owned());
        };
        let requests = &details.review_requests;
        if !requests.items.is_empty() {
            return Finding::No;
        }
        if !requests.complete {
            return Finding::Unknown("some review requests could not be read".to_owned());
        }
        let mut reasons = Vec::new();
        if let Approval::NotApproved(why) = approval {
            reasons.push(why.clone());
        }
        if let Some(awaiting) = self.awaiting_text() {
            reasons.push(awaiting);
        }
        if !reasons.is_empty() {
            return Finding::Holds(reasons.join("; "));
        }
        if matches!(approval, Approval::Unknown) {
            return Finding::Unknown(
                "whether to ask for review, since approval could not be established".to_owned(),
            );
        }
        if let Some(why) = self.reviews_unread() {
            return Finding::Unknown(format!("{why}, so someone may be awaiting re-review"));
        }
        Finding::No
    }

    /// What stops Merge under the workflow, beyond milestone 1's blockers.
    fn merge_blockers(&self) -> Vec<String> {
        let mut blockers: Vec<String> = self.changes_at_head_text().into_iter().collect();
        if let Some(awaiting) = self.awaiting_text() {
            blockers.push(awaiting);
        } else if let Some(why) = self.reviews_unread().filter(|_| self.pr.details.is_some()) {
            blockers.push(format!("{why}, so someone may be awaiting re-review"));
        }
        blockers
    }

    /// "changes requested at the current head by bob (workflow policy: …)".
    fn changes_at_head_text(&self) -> Option<String> {
        let by = &self.read.changes_at_head;
        (!by.is_empty()).then(|| {
            format!(
                "changes requested at the current head by {} ({AT_HEAD})",
                by.join(", ")
            )
        })
    }

    /// Context that is no rule of its own: changes requested on an older head
    /// by someone not awaiting re-review.
    fn notes(&self) -> Vec<String> {
        if self.read.changes_earlier.is_empty() {
            return Vec::new();
        }
        vec![format!(
            "changes requested on an older head by {} ({AT_HEAD})",
            self.read.changes_earlier.join(", ")
        )]
    }

    /// "awaiting re-review: bob requested changes on an older head, carol
    /// commented on an older head, and neither was asked again (…)".
    fn awaiting_text(&self) -> Option<String> {
        let awaiting = &self.read.awaiting;
        if awaiting.is_empty() {
            return None;
        }
        let entries: Vec<String> = awaiting
            .iter()
            .map(|a| {
                let what = if a.commented_only {
                    "commented on"
                } else {
                    "requested changes on"
                };
                format!("{} {what} an older head", a.login)
            })
            .collect();
        let mut policies = vec![FRESH_LOOK];
        if awaiting.iter().any(|a| a.commented_only) {
            policies.push(COMMENTS);
        }
        let asked = if awaiting.len() == 1 {
            "and hasn't been asked again"
        } else {
            "and none has been asked again"
        };
        Some(format!(
            "awaiting re-review: {} {asked} ({})",
            entries.join(", "),
            policies.join("; ")
        ))
    }
}

/// One PR's read details, for the definitions above.
struct Reading<'a> {
    pr: &'a PullRequest,
    details: &'a PrDetails,
    exact_head: bool,
}

impl Reading<'_> {
    fn at_head(&self, review: &Review) -> bool {
        review
            .commit
            .as_deref()
            .is_some_and(|commit| commit.eq_ignore_ascii_case(&self.pr.head_commit))
    }

    fn is_author(&self, login: &str) -> bool {
        self.pr
            .author
            .as_deref()
            .is_some_and(|author| same_login(author, login))
    }

    fn requested(&self, login: &str) -> bool {
        requested(
            &self.details.review_requests.items,
            &Reviewer::User(login.to_owned()),
        )
    }

    /// `login`'s latest review of any kind is on the current head.
    fn looked_at_head(&self, login: &str) -> bool {
        self.latest().iter().any(|review| {
            review
                .author
                .as_deref()
                .is_some_and(|author| same_login(author, login))
                && self.at_head(review)
        })
    }

    fn standing(&self) -> &[Review] {
        &self.details.latest_reviews.items
    }

    fn latest(&self) -> &[Review] {
        &self.details.latest_reviews_with_comments.items
    }

    /// Users whose standing approval counts.
    fn fresh_approvers(&self) -> Vec<String> {
        let mut approvers = Vec::new();
        for review in self.standing() {
            let Some(login) = review.author.as_deref() else {
                continue;
            };
            let counts = review.verdict == ReviewVerdict::Approved
                && review.author_kind == ActorKind::User
                && !self.is_author(login)
                && (!self.exact_head || self.at_head(review));
            if counts {
                push_login(&mut approvers, login);
            }
        }
        approvers
    }

    /// Approvals that don't count with exact-head approval on, and whether
    /// one of them is a bot's.
    fn discounted(&self, fresh: &[String]) -> (Vec<String>, bool) {
        if !self.exact_head {
            return (Vec::new(), false);
        }
        let mut discounted = Vec::new();
        let mut bot = false;
        for review in self.standing() {
            if review.verdict != ReviewVerdict::Approved {
                continue;
            }
            let login = review.author.as_deref().unwrap_or(DELETED_ACCOUNT);
            if fresh.iter().any(|f| same_login(f, login)) {
                continue;
            }
            let why = match review.author_kind {
                ActorKind::Bot => {
                    bot = true;
                    format!("{login} is a bot")
                }
                ActorKind::Other => format!("{login} is not a user account"),
                ActorKind::User => format!("{login} approved an older head"),
            };
            discounted.push(why);
        }
        (discounted, bot)
    }

    /// Users awaiting re-review, in the order their reviews were listed.
    fn awaiting(&self, fresh: &[String], comments_count: bool) -> Vec<Awaiting> {
        let mut awaiting: Vec<Awaiting> = Vec::new();
        for review in self.standing().iter().chain(self.latest()) {
            let Some(login) = review.author.as_deref() else {
                continue;
            };
            if review.author_kind != ActorKind::User
                || self.at_head(review)
                || self.is_author(login)
                || self.requested(login)
                || fresh.iter().any(|f| same_login(f, login))
                || self.looked_at_head(login)
            {
                continue;
            }
            let commented_only = match review.verdict {
                ReviewVerdict::ChangesRequested => false,
                ReviewVerdict::Commented if comments_count => true,
                _ => continue,
            };
            match awaiting.iter_mut().find(|a| same_login(&a.login, login)) {
                // Changes requested outweigh a comment.
                Some(entry) => entry.commented_only &= commented_only,
                None => awaiting.push(Awaiting {
                    login: login.to_owned(),
                    commented_only,
                }),
            }
        }
        awaiting
    }

    /// Who has a standing change request at the current head (`at_head`) or
    /// on an older one.
    fn changes_requested(&self, at_head: bool) -> Vec<String> {
        let mut by = Vec::new();
        for review in self.standing() {
            if review.verdict == ReviewVerdict::ChangesRequested && self.at_head(review) == at_head
            {
                push_login(&mut by, review.author.as_deref().unwrap_or(DELETED_ACCOUNT));
            }
        }
        by
    }
}

/// GitHub's own review decision, which exact-head approval tightens but never
/// overrules: REVIEW_REQUIRED means branch protection wants an approval den's
/// rules don't see (a second one, or a code owner's).
fn github_decision(pr: &PullRequest) -> Approval {
    match pr.review_decision {
        ReviewDecision::Approved | ReviewDecision::NotRequired => Approval::Approved,
        ReviewDecision::ReviewRequired => Approval::NotApproved(
            "GitHub reports that a review is still required (for example a second approval or a code owner's)"
                .to_owned(),
        ),
        ReviewDecision::ChangesRequested => {
            Approval::NotApproved("GitHub reports changes requested".to_owned())
        }
        ReviewDecision::Unknown => Approval::Unknown,
    }
}

fn push_login(logins: &mut Vec<String>, login: &str) {
    if !logins.iter().any(|l| same_login(l, login)) {
        logins.push(login.to_owned());
    }
}

/// The configured reviewers plus those awaiting re-review, minus fresh
/// approvers, pending requests and the author. `requests` is `None` when the
/// review requests could not be read.
fn suggested(
    configured: &[Reviewer],
    awaiting: &[String],
    pr: &PullRequest,
    requests: Option<&[Reviewer]>,
    fresh: &[String],
) -> Vec<Reviewer> {
    let candidates = configured
        .iter()
        .cloned()
        .chain(awaiting.iter().cloned().map(Reviewer::User));
    let mut suggested: Vec<Reviewer> = Vec::new();
    for candidate in candidates {
        if suggested.iter().any(|s| same_reviewer(s, &candidate)) {
            continue;
        }
        let excluded = requests.is_some_and(|requests| requested(requests, &candidate))
            || match &candidate {
                Reviewer::User(login) => {
                    pr.author
                        .as_deref()
                        .is_some_and(|author| same_login(author, login))
                        || fresh.iter().any(|f| same_login(f, login))
                }
                Reviewer::Team(_) => false,
            };
        if !excluded {
            suggested.push(candidate);
        }
    }
    suggested
}

/// `reviewer` has a pending review request among `requests`.
fn requested(requests: &[Reviewer], reviewer: &Reviewer) -> bool {
    requests.iter().any(|r| same_reviewer(r, reviewer))
}

/// The details were read at the PR list's head, so their reviews' commits can
/// be compared against it.
fn same_head(pr: &PullRequest, details: &PrDetails) -> bool {
    details
        .head_commit
        .as_deref()
        .is_some_and(|head| head.eq_ignore_ascii_case(&pr.head_commit))
}

fn same_reviewer(a: &Reviewer, b: &Reviewer) -> bool {
    match (a, b) {
        (Reviewer::User(a), Reviewer::User(b)) => same_login(a, b),
        (Reviewer::Team(a), Reviewer::Team(b)) => a.same(b),
        _ => false,
    }
}
