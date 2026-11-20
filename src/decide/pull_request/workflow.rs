//! The Linear Workflow's review rules for one PR (spec 0002, "Next Step rules:
//! PRs of an Owner with a Linear Workflow"). Only PRs of an Owner with a
//! Linear Workflow reach this module; every other PR keeps milestone 1's
//! rules in the parent module.
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
//! - **Awaiting re-review**: a user other than the author, not currently
//!   requested and not a fresh approver, whose standing or latest review
//!   requested changes on an older head, or whose latest review only commented
//!   on an older head when comment reviews need re-review. A review whose
//!   commit GitHub no longer has counts as on an older head.
//! - **Address review** holds on unresolved threads, or on changes requested
//!   at the current head (by anyone); changes requested on an older head fall
//!   through.
//! - **Suggested reviewers**: the configured reviewers plus those awaiting
//!   re-review, minus fresh approvers, pending requests and the author.

use super::{Approval, Finding, approval as github_approval, unresolved_threads};
use crate::domain::{
    ReviewPolicy, TeamRef,
    facts::{ActorKind, PrDetails, PullRequest, Review, ReviewVerdict, Reviewer},
    ids::same_login,
};

const EXACT_HEAD: &str = "workflow policy: approval must be at the current head";
const NO_BOTS: &str = "workflow policy: bots' approvals don't count";
const FRESH_LOOK: &str = "workflow policy: feedback given on an older head needs a fresh look";
const COMMENTS: &str = "workflow policy: comment reviews need re-review";
const AT_HEAD: &str = "workflow policy: only changes requested at the current head need addressing";
const DELETED_ACCOUNT: &str = "a deleted account";

/// What the workflow's review rules found for one PR.
pub(super) struct WorkflowReview {
    exact_head: bool,
    /// The details query was read (checks, threads, requests and reviews).
    details_read: bool,
    /// Every standing review was read.
    standing_complete: bool,
    /// Every standing and every latest review was read.
    reviews_complete: bool,
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
    suggested: Vec<Reviewer>,
}

/// A user whose feedback needs a fresh look.
struct Awaiting {
    login: String,
    commented_only: bool,
}

impl WorkflowReview {
    pub(super) fn assess(pr: &PullRequest, policy: &ReviewPolicy) -> Self {
        let Some(details) = &pr.details else {
            return Self {
                exact_head: policy.exact_head_approval,
                details_read: false,
                standing_complete: false,
                reviews_complete: false,
                fresh_approvers: Vec::new(),
                discounted: Vec::new(),
                bot_discounted: false,
                awaiting: Vec::new(),
                changes_at_head: Vec::new(),
                changes_earlier: Vec::new(),
                suggested: suggested(&policy.reviewers, &[], pr, None),
            };
        };
        let read = Reading {
            pr,
            details,
            exact_head: policy.exact_head_approval,
        };
        let fresh_approvers = read.fresh_approvers();
        let (discounted, bot_discounted) = read.discounted(&fresh_approvers);
        let awaiting = read.awaiting(&fresh_approvers, policy.comment_reviews_need_rereview);
        let changes_earlier = read
            .changes_requested(false)
            .into_iter()
            .filter(|login| !awaiting.iter().any(|a| same_login(&a.login, login)))
            .collect();
        let waiting_logins: Vec<String> = awaiting.iter().map(|a| a.login.clone()).collect();
        Self {
            exact_head: policy.exact_head_approval,
            details_read: true,
            standing_complete: details.latest_reviews.complete,
            reviews_complete: details.latest_reviews.complete
                && details.latest_reviews_with_comments.complete,
            changes_at_head: read.changes_requested(true),
            changes_earlier,
            suggested: suggested(
                &policy.reviewers,
                &waiting_logins,
                pr,
                Some((details, &fresh_approvers)),
            ),
            fresh_approvers,
            discounted,
            bot_discounted,
            awaiting,
        }
    }

    /// The suggested-reviewers set.
    pub(super) fn suggested_reviewers(&self) -> Vec<Reviewer> {
        self.suggested.clone()
    }

    /// Whether the PR is approved under the workflow: a fresh approval with
    /// exact-head approval on, GitHub's review decision with it off.
    pub(super) fn approval(&self, pr: &PullRequest) -> Approval {
        if !self.exact_head {
            return github_approval(pr);
        }
        if !self.details_read {
            return Approval::Unknown;
        }
        if !self.fresh_approvers.is_empty() {
            return Approval::Approved;
        }
        if !self.standing_complete {
            return Approval::Unknown;
        }
        let mut policies = vec![EXACT_HEAD];
        if self.bot_discounted {
            policies.push(NO_BOTS);
        }
        let why = if self.discounted.is_empty() {
            format!("no approval at the current head ({})", policies.join("; "))
        } else {
            format!(
                "no approval at the current head: {} ({})",
                self.discounted.join(", "),
                policies.join("; ")
            )
        };
        Approval::NotApproved(why)
    }

    /// "approved at the current head by alice (workflow policy: …)", or
    /// milestone 1's words when exact-head approval is off.
    pub(super) fn approved_text(&self, approval: &Approval) -> String {
        if self.exact_head {
            return format!(
                "approved at the current head by {} ({EXACT_HEAD})",
                self.fresh_approvers.join(", ")
            );
        }
        match approval {
            Approval::NoReviewRequired => "no review required and no changes requested".to_owned(),
            _ => "approved".to_owned(),
        }
    }

    /// The review half of Merge's winning rule.
    pub(super) fn merge_text(&self, approval: &Approval) -> String {
        format!(
            "{}, nobody awaiting re-review ({FRESH_LOOK})",
            self.approved_text(approval)
        )
    }

    /// Address review: unresolved threads, or changes requested at the
    /// current head. Changes requested on an older head fall through.
    pub(super) fn address_review(&self, pr: &PullRequest) -> Finding {
        let Some(details) = &pr.details else {
            return Finding::Unknown("review threads could not be read".to_owned());
        };
        let mut reasons = Vec::new();
        if !self.changes_at_head.is_empty() {
            reasons.push(format!(
                "changes requested at the current head by {} ({AT_HEAD})",
                self.changes_at_head.join(", ")
            ));
        }
        reasons.extend(unresolved_threads(details));
        if !reasons.is_empty() {
            return Finding::Holds(reasons.join(", "));
        }
        if !details.threads_complete {
            return Finding::Unknown("not every review thread was read".to_owned());
        }
        if !self.standing_complete {
            return Finding::Unknown("not every review was read".to_owned());
        }
        Finding::No
    }

    /// Request review: no fresh approval, or someone awaiting re-review, and
    /// no review request pending.
    pub(super) fn request_review(&self, pr: &PullRequest, approval: &Approval) -> Finding {
        let Some(details) = &pr.details else {
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
        if !self.reviews_complete {
            return Finding::Unknown(
                "not every review was read, so someone may be awaiting re-review".to_owned(),
            );
        }
        Finding::No
    }

    /// What stops Merge under the workflow, beyond milestone 1's blockers.
    pub(super) fn merge_blockers(&self) -> Vec<String> {
        if let Some(awaiting) = self.awaiting_text() {
            return vec![awaiting];
        }
        if self.details_read && !self.reviews_complete {
            return vec![
                "not every review was read, so someone may be awaiting re-review".to_owned(),
            ];
        }
        Vec::new()
    }

    /// Context that is no rule of its own: changes requested on an older head
    /// by someone not awaiting re-review.
    pub(super) fn notes(&self) -> Vec<String> {
        if self.changes_earlier.is_empty() {
            return Vec::new();
        }
        vec![format!(
            "changes requested on an older head by {} ({AT_HEAD})",
            self.changes_earlier.join(", ")
        )]
    }

    /// "awaiting re-review: bob requested changes on an older head, carol
    /// commented on an older head, and neither was asked again (…)".
    fn awaiting_text(&self) -> Option<String> {
        if self.awaiting.is_empty() {
            return None;
        }
        let entries: Vec<String> = self
            .awaiting
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
        if self.awaiting.iter().any(|a| a.commented_only) {
            policies.push(COMMENTS);
        }
        let asked = if self.awaiting.len() == 1 {
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
        self.details
            .review_requests
            .items
            .iter()
            .any(|r| matches!(r, Reviewer::User(user) if same_login(user, login)))
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

fn push_login(logins: &mut Vec<String>, login: &str) {
    if !logins.iter().any(|l| same_login(l, login)) {
        logins.push(login.to_owned());
    }
}

/// The configured reviewers plus those awaiting re-review, minus fresh
/// approvers, pending requests and the author. `read` is `None` when the
/// details (requests and reviews) could not be read: then only the author is
/// taken out.
fn suggested(
    configured: &[Reviewer],
    awaiting: &[String],
    pr: &PullRequest,
    read: Option<(&PrDetails, &[String])>,
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
        let excluded = match &candidate {
            Reviewer::User(login) => {
                pr.author
                    .as_deref()
                    .is_some_and(|author| same_login(author, login))
                    || read.is_some_and(|(details, fresh)| {
                        fresh.iter().any(|f| same_login(f, login)) || requested(details, &candidate)
                    })
            }
            Reviewer::Team(_) => read.is_some_and(|(details, _)| requested(details, &candidate)),
        };
        if !excluded {
            suggested.push(candidate);
        }
    }
    suggested
}

fn requested(details: &PrDetails, reviewer: &Reviewer) -> bool {
    details
        .review_requests
        .items
        .iter()
        .any(|r| same_reviewer(r, reviewer))
}

fn same_reviewer(a: &Reviewer, b: &Reviewer) -> bool {
    match (a, b) {
        (Reviewer::User(a), Reviewer::User(b)) => same_login(a, b),
        (Reviewer::Team(a), Reviewer::Team(b)) => same_team(a, b),
        _ => false,
    }
}

fn same_team(a: &TeamRef, b: &TeamRef) -> bool {
    same_login(&a.org, &b.org) && a.slug.eq_ignore_ascii_case(&b.slug)
}
