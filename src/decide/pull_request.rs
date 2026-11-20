//! Pull Request rules. The first rule that holds wins:
//!
//! 1. draft → Finish draft
//! 2. conflicting → Resolve conflicts
//! 3. any failing check (required or advisory) → Fix CI
//! 4. changes requested, or unresolved review threads → Address review
//! 5. review requested → Review
//! 6. approved (or, where the base branch requires no review, nobody asked
//!    for changes), no unresolved threads and every review request read, every
//!    check passing, known mergeable, not queued, and GitHub itself reporting
//!    the PR ready to merge → Merge
//!
//! A rule whose facts den could not establish neither holds nor is ruled out:
//! later rules may still win, and the Explanation says which earlier rule
//! could not be ruled out. Merge needs every one of its facts known, so an
//! unknown never yields Merge. With no rule holding, the PR has no Next Step
//! and the Explanation says what stops it from merging.
//!
//! Waiting On: the PR author, except for Review (the requested reviewers;
//! me when I or a team of mine is requested; unattributed when den read too
//! little of the requests or of my teams to say it is not me).

use super::{Attribution, Context, Decision, Explanation, WorkItem};
use crate::domain::{
    NextStep, Source, SourceState, Viewer,
    facts::{
        Check, CheckOutcome, MergeState, Mergeability, PrDetails, PullRequest, ReviewDecision,
        ReviewVerdict, Reviewer,
    },
};

const AUTHOR_ROLE: &str = "the PR author";
const REVIEWERS_ROLE: &str = "the requested reviewers";
/// Failing or pending checks named in one line before "and N more".
const MAX_NAMED_CHECKS: usize = 4;

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .pull_requests
        .iter()
        .map(|pr| decide(ctx, pr))
        .collect()
}

/// What one rule found.
enum Finding {
    /// The rule holds; the text says why ("conflicts with main").
    Holds(String),
    /// The rule does not hold.
    No,
    /// den could not establish whether it holds; the text says what is missing.
    Unknown(String),
}

/// Whether the PR is approved, as far as den can tell.
enum Approval {
    Approved,
    /// GitHub reports no review decision (the base branch requires no review),
    /// nobody's latest review approves or requests changes, and every latest
    /// review was read. Enough for Merge (an orchestrator decision; the spec
    /// says "approved").
    NoReviewRequired,
    NotApproved(&'static str),
    Unknown,
}

fn decide(ctx: &Context<'_>, pr: &PullRequest) -> Decision {
    let item = WorkItem {
        id: pr.id(),
        title: pr.title.clone(),
        url: pr.url.clone(),
        updated_at: pr.updated_at,
    };
    let approval = approval(pr);
    let merge_blockers = merge_blockers(pr, &approval);
    let rules = [
        (NextStep::FinishDraft, draft(pr)),
        (NextStep::ResolveConflicts, conflicts(pr)),
        (NextStep::FixCi, fix_ci(pr.details.as_ref())),
        (NextStep::AddressReview, address_review(pr)),
        (NextStep::Review, review(pr.details.as_ref(), ctx.viewer)),
        (NextStep::Merge, merge(pr, &approval, &merge_blockers)),
    ];
    let winner = rules
        .iter()
        .enumerate()
        .find_map(|(i, (step, finding))| match finding {
            Finding::Holds(why) => Some((i, *step, why.as_str())),
            _ => None,
        });
    let winner_index = winner.map(|(i, _, _)| i);
    let winner_step = winner.map(|(_, step, _)| step);

    let mut other_conditions: Vec<String> = rules
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != winner_index)
        .filter_map(|(_, (_, finding))| match finding {
            Finding::Holds(why) => Some(why.clone()),
            _ => None,
        })
        .collect();
    other_conditions.extend(notes(pr, &approval, &merge_blockers, winner_step));

    let mut unknowns: Vec<String> = rules
        .iter()
        .filter_map(|(_, finding)| match finding {
            Finding::Unknown(what) => Some(what.clone()),
            _ => None,
        })
        .collect();
    unknowns.extend(unrecognised_checks(pr.details.as_ref()));
    if matches!(approval, Approval::Unknown) {
        unknowns.push("approval could not be established".to_owned());
    }
    // Unknown mergeability already says this, through the conflicts rule.
    if pr.merge_state == MergeState::Unknown && pr.mergeability != Mergeability::Unknown {
        unknowns.push("merge state not computed yet by GitHub".to_owned());
    }
    unknowns.dedup();

    let Some((index, step, why)) = winner else {
        return Decision::new(
            ctx,
            item,
            None,
            Attribution::nobody("no Next Step, so nobody owes one"),
            Explanation {
                no_step_reason: Some(format!("no rule matched: {}", merge_blockers.join("; "))),
                other_conditions,
                unknowns,
                ..Explanation::default()
            },
        );
    };
    let not_ruled_out: Vec<String> = rules[..index]
        .iter()
        .filter_map(|(earlier, finding)| match finding {
            Finding::Unknown(what) => Some(format!("{} ({what})", earlier.label())),
            _ => None,
        })
        .collect();
    let winning_rule = if not_ruled_out.is_empty() {
        why.to_owned()
    } else {
        format!(
            "{why}; earlier rules not ruled out: {}",
            not_ruled_out.join(", ")
        )
    };
    Decision::new(
        ctx,
        item,
        Some(step),
        waiting_on(step, pr, ctx),
        Explanation {
            winning_rule: Some(winning_rule),
            other_conditions,
            unknowns,
            ..Explanation::default()
        },
    )
}

fn waiting_on(step: NextStep, pr: &PullRequest, ctx: &Context<'_>) -> Attribution {
    let viewer = ctx.viewer;
    if step != NextStep::Review {
        return Attribution::person(viewer, pr.author.as_deref(), AUTHOR_ROLE);
    }
    let requests = pr
        .details
        .as_ref()
        .map(|d| d.review_requests.items.as_slice())
        .unwrap_or_default();
    let mut users = Vec::new();
    let mut teams = Vec::new();
    for reviewer in requests {
        match reviewer {
            Reviewer::User(login) => users.push(login.clone()),
            Reviewer::Team(team) => teams.push(team.clone()),
        }
    }
    // What den did not read that could make the review mine.
    let mut unread = Vec::new();
    match &pr.details {
        None => unread.push("the review requests could not be read".to_owned()),
        Some(details) if !details.review_requests.complete => {
            unread.push("not every review request was read".to_owned());
        }
        Some(_) => {}
    }
    let my_teams_known =
        ctx.observation.coverage.get(&Source::Viewer) == Some(&SourceState::Complete);
    if !teams.is_empty() && !my_teams_known {
        unread.push("your team memberships were not read in full".to_owned());
    }
    Attribution::any_of(viewer, &users, &teams, REVIEWERS_ROLE, &unread)
}

fn draft(pr: &PullRequest) -> Finding {
    if pr.is_draft {
        Finding::Holds("the PR is a draft".to_owned())
    } else {
        Finding::No
    }
}

fn conflicts(pr: &PullRequest) -> Finding {
    match pr.mergeability {
        Mergeability::Conflicting => Finding::Holds(format!("conflicts with {}", pr.base_ref)),
        Mergeability::Unknown => {
            Finding::Unknown("mergeability not computed yet by GitHub".to_owned())
        }
        Mergeability::Mergeable => Finding::No,
    }
}

fn fix_ci(details: Option<&PrDetails>) -> Finding {
    let Some(details) = details else {
        return Finding::Unknown("checks could not be read".to_owned());
    };
    let failing: Vec<&Check> = details
        .checks
        .items
        .iter()
        .filter(|c| c.outcome == CheckOutcome::Failing)
        .collect();
    if !failing.is_empty() {
        return Finding::Holds(describe_failing(&failing));
    }
    if !details.checks.complete {
        return Finding::Unknown("GitHub listed more checks than den read".to_owned());
    }
    Finding::No
}

fn address_review(pr: &PullRequest) -> Finding {
    let mut reasons = Vec::new();
    if changes_requested(pr) {
        reasons.push(changes_requested_by(pr.details.as_ref()));
    }
    let Some(details) = &pr.details else {
        if reasons.is_empty() {
            return Finding::Unknown("review threads could not be read".to_owned());
        }
        return Finding::Holds(reasons.join(", "));
    };
    if details.unresolved_threads > 0 {
        let at_least = if details.threads_complete {
            ""
        } else {
            "at least "
        };
        reasons.push(format!(
            "{at_least}{} unresolved review {}",
            details.unresolved_threads,
            plural(details.unresolved_threads as usize, "thread")
        ));
    }
    if !reasons.is_empty() {
        return Finding::Holds(reasons.join(", "));
    }
    if !details.threads_complete {
        return Finding::Unknown("not every review thread was read".to_owned());
    }
    Finding::No
}

fn review(details: Option<&PrDetails>, viewer: &Viewer) -> Finding {
    let Some(details) = details else {
        return Finding::Unknown("review requests could not be read".to_owned());
    };
    let requests = &details.review_requests;
    if requests.items.is_empty() {
        if requests.complete {
            return Finding::No;
        }
        return Finding::Unknown("some review requests could not be read".to_owned());
    }
    let names: Vec<String> = requests
        .items
        .iter()
        .map(|reviewer| match reviewer {
            Reviewer::User(login) if viewer.is(login) => "you".to_owned(),
            Reviewer::User(login) => login.clone(),
            Reviewer::Team(team) if viewer.belongs_to(team) => {
                format!("your team {}/{}", team.org, team.slug)
            }
            Reviewer::Team(team) => format!("team {}/{}", team.org, team.slug),
        })
        .collect();
    Finding::Holds(format!("review requested from {}", names.join(", ")))
}

fn merge(pr: &PullRequest, approval: &Approval, blockers: &[String]) -> Finding {
    if !blockers.is_empty() {
        return Finding::No;
    }
    let review = match approval {
        Approval::NoReviewRequired => "no review required and no changes requested",
        _ => "approved",
    };
    Finding::Holds(format!(
        "{review}, checks passing and mergeable into {}",
        pr.base_ref
    ))
}

/// Everything that stops Merge. Empty means Merge holds: approved (or no
/// review required), no unresolved review threads and all threads read, every
/// review request read, every check (required or advisory) passing and all
/// read, known mergeable, not a draft, not in a merge queue, and GitHub's own
/// merge state agreeing.
fn merge_blockers(pr: &PullRequest, approval: &Approval) -> Vec<String> {
    let mut blockers = Vec::new();
    if pr.is_draft {
        blockers.push("the PR is a draft".to_owned());
    }
    match approval {
        Approval::Approved | Approval::NoReviewRequired => {}
        Approval::NotApproved(why) => blockers.push((*why).to_owned()),
        Approval::Unknown => blockers.push("approval could not be established".to_owned()),
    }
    match &pr.details {
        None => blockers.push("checks could not be read".to_owned()),
        Some(details) => {
            if details.unresolved_threads > 0 {
                blockers.push("unresolved review threads".to_owned());
            } else if !details.threads_complete {
                blockers.push("not every review thread was read".to_owned());
            }
            // An unread request could be a pending review, which comes before Merge.
            if !details.review_requests.complete {
                blockers.push("not every review request was read".to_owned());
            }
            let checks = &details.checks.items;
            if checks.iter().any(|c| c.outcome == CheckOutcome::Failing) {
                blockers.push("checks failing".to_owned());
            }
            let pending: Vec<&Check> = checks
                .iter()
                .filter(|c| c.outcome == CheckOutcome::Pending)
                .collect();
            if !pending.is_empty() {
                blockers.push(describe_pending(&pending));
            }
            if checks
                .iter()
                .any(|c| matches!(c.outcome, CheckOutcome::Unrecognised(_)))
            {
                blockers.push("a check is in a state den does not recognise".to_owned());
            }
            if !details.checks.complete {
                blockers.push("not every check was read".to_owned());
            }
        }
    }
    match pr.mergeability {
        Mergeability::Mergeable => {}
        Mergeability::Conflicting => blockers.push(format!("conflicts with {}", pr.base_ref)),
        Mergeability::Unknown => {
            blockers.push("waiting for GitHub to compute mergeability".to_owned());
        }
    }
    if pr.in_merge_queue {
        blockers.push("already in the merge queue".to_owned());
    }
    // Only when den's own facts allow Merge: GitHub's merge state then catches
    // rules den does not read. Otherwise den's reasons already say more.
    if blockers.is_empty()
        && let Some(blocker) = merge_state_blocker(pr)
    {
        blockers.push(blocker);
    }
    blockers
}

/// Why GitHub's own merge state disagrees with Merge, if it does.
fn merge_state_blocker(pr: &PullRequest) -> Option<String> {
    let base = &pr.base_ref;
    match pr.merge_state {
        MergeState::Clean | MergeState::HasHooks => None,
        MergeState::Blocked => Some(
            "GitHub reports merging blocked by a rule den does not read \
             (for example a required check that has not reported)"
                .to_owned(),
        ),
        MergeState::Behind => Some(format!(
            "behind {base}, which must be merged in before this PR can merge"
        )),
        MergeState::Unstable => Some("GitHub reports a check that is not passing".to_owned()),
        MergeState::Dirty => Some(format!(
            "GitHub reports it cannot merge cleanly into {base}"
        )),
        MergeState::Draft => Some("GitHub reports the PR as a draft".to_owned()),
        MergeState::Unknown => {
            Some("waiting for GitHub to compute whether it can merge".to_owned())
        }
    }
}

/// Context that is not itself a rule: checks still running, merge queue,
/// approval. Skipped where the winning rule already says it.
fn notes(
    pr: &PullRequest,
    approval: &Approval,
    merge_blockers: &[String],
    winner: Option<NextStep>,
) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(details) = &pr.details {
        let pending: Vec<&Check> = details
            .checks
            .items
            .iter()
            .filter(|c| c.outcome == CheckOutcome::Pending)
            .collect();
        if !pending.is_empty() && winner.is_some() {
            notes.push(describe_pending(&pending));
        }
    }
    if pr.in_merge_queue && winner.is_some() {
        notes.push("in the merge queue".to_owned());
    }
    // When Merge holds (behind an earlier rule), its own condition says "approved".
    if matches!(approval, Approval::Approved) && winner.is_some() && !merge_blockers.is_empty() {
        notes.push("approved".to_owned());
    }
    notes
}

fn unrecognised_checks(details: Option<&PrDetails>) -> Vec<String> {
    details
        .map(|d| d.checks.items.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| match &c.outcome {
            CheckOutcome::Unrecognised(state) => Some(format!(
                "check {} reported \"{state}\", which den does not recognise",
                c.name
            )),
            _ => None,
        })
        .collect()
}

fn approval(pr: &PullRequest) -> Approval {
    match pr.review_decision {
        ReviewDecision::Approved => Approval::Approved,
        ReviewDecision::ChangesRequested => Approval::NotApproved("changes requested"),
        ReviewDecision::ReviewRequired => {
            Approval::NotApproved("not approved: a review is required")
        }
        // Without GitHub's decision, a latest review asking for changes still
        // counts; nothing else can establish approval.
        ReviewDecision::Unknown if latest_review_requests_changes(pr) => {
            Approval::NotApproved("changes requested")
        }
        ReviewDecision::Unknown => Approval::Unknown,
        ReviewDecision::NotRequired => {
            let Some(details) = &pr.details else {
                return Approval::Unknown;
            };
            let reviews = &details.latest_reviews;
            if latest_review_requests_changes(pr) {
                Approval::NotApproved("changes requested")
            } else if !reviews.complete {
                Approval::Unknown
            } else if reviews
                .items
                .iter()
                .any(|r| r.verdict == ReviewVerdict::Approved)
            {
                Approval::Approved
            } else {
                Approval::NoReviewRequired
            }
        }
    }
}

/// GitHub's decision when it gives one; otherwise anyone's latest review.
fn changes_requested(pr: &PullRequest) -> bool {
    match pr.review_decision {
        ReviewDecision::NotRequired | ReviewDecision::Unknown => latest_review_requests_changes(pr),
        decision => decision == ReviewDecision::ChangesRequested,
    }
}

fn latest_review_requests_changes(pr: &PullRequest) -> bool {
    pr.details.as_ref().is_some_and(|d| {
        d.latest_reviews
            .items
            .iter()
            .any(|r| r.verdict == ReviewVerdict::ChangesRequested)
    })
}

fn changes_requested_by(details: Option<&PrDetails>) -> String {
    let by: Vec<&str> = details
        .map(|d| d.latest_reviews.items.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|r| r.verdict == ReviewVerdict::ChangesRequested)
        .filter_map(|r| r.author.as_deref())
        .collect();
    match by.as_slice() {
        [] => "changes requested".to_owned(),
        names => format!("changes requested by {}", names.join(", ")),
    }
}

/// "2 failing checks, 1 required and 1 advisory: build (required), lint (advisory)".
fn describe_failing(failing: &[&Check]) -> String {
    let total = failing.len();
    let required = failing.iter().filter(|c| c.required).count();
    let advisory = total - required;
    let checks = plural(total, "check");
    let summary = match (required, advisory) {
        (0, _) => format!("{total} failing {checks}, advisory only"),
        (_, 0) => format!("{total} failing required {checks}"),
        _ => format!("{total} failing {checks}, {required} required and {advisory} advisory"),
    };
    format!(
        "{summary}: {}",
        named(failing, |c| format!(
            "{} ({})",
            c.name,
            if c.required { "required" } else { "advisory" }
        ))
    )
}

/// "checks running: 2 pending (build, test)".
fn describe_pending(pending: &[&Check]) -> String {
    format!(
        "checks running: {} pending ({})",
        pending.len(),
        named(pending, |c| c.name.clone())
    )
}

fn named(checks: &[&Check], name: impl Fn(&Check) -> String) -> String {
    let mut names: Vec<String> = checks
        .iter()
        .take(MAX_NAMED_CHECKS)
        .map(|c| name(c))
        .collect();
    if checks.len() > MAX_NAMED_CHECKS {
        names.push(format!("and {} more", checks.len() - MAX_NAMED_CHECKS));
    }
    names.join(", ")
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_owned()
    } else {
        format!("{noun}s")
    }
}
