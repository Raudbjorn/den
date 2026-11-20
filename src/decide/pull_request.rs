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
//!
//! For an Owner with a Linear Workflow, the policy's review rules
//! ([`workflow`]) change approval and Address review, and add **Request
//! review** between Review and Merge (Waiting On the author). Which review
//! rules apply is settled once, in [`ReviewFindings::assess`]; the rules
//! below read its findings and never ask which policy produced them. A PR
//! that would Merge then passes [`held_back`], the one place "would Merge"
//! becomes Merge: in a Held Repo it has no Next Step and is held for release;
//! an attached member of a Landing Group of its own Owner has no Next Step
//! and "lands with" its Linear Issue (#37), which offers Land when the whole
//! group is ready. When den couldn't read the Linear Issue lists of a Merge
//! PR's Owner (or one of their issues' attachments) completely, the PR might
//! be a member den didn't see: it still shows Merge, but those Linear sources
//! join its coverage and the Explanation says what den couldn't establish.
//! The empty policy (no Linear Workflow) changes none of this module's
//! Decisions.

mod workflow;

use super::{
    Attribution, Context, Decision, Explanation, HeldBack, WorkItem, correlation::GroupRef,
};
use crate::domain::{
    CoverageEntry, Incompleteness, LinearSourceKind, LinearWorkflowPolicy, NextStep, ReviewPolicy,
    Source, SourceKind, SourceState, Viewer,
    facts::{
        AttachmentsRead, Check, CheckOutcome, LinearIssue, MergeState, Mergeability, PrDetails,
        PullRequest, ReviewDecision, ReviewVerdict, Reviewer,
    },
    ids::same_login,
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
    NotApproved(String),
    Unknown,
}

/// What a PR's review rules found: milestone 1's, or those of its Owner's
/// Linear Workflow. Everything a review policy changes about a PR's Decision
/// is here, so the rules in [`decide`] apply whichever policy without
/// branching on it.
struct ReviewFindings {
    approval: Approval,
    /// Address review's finding.
    address_review: Finding,
    /// Request review's finding; `None` where there is no such step (an
    /// Owner without a Linear Workflow).
    request_review: Option<Finding>,
    /// How approval reads in a note, when Merge holds behind an earlier rule.
    approved: String,
    /// The review half of Merge's winning rule.
    merge: String,
    /// What else stops Merge on review grounds, beyond approval.
    merge_blockers: Vec<String>,
    /// Context that is no rule of its own.
    notes: Vec<String>,
    /// Who to ask on Request review ([`Decision::suggested_reviewers`]).
    suggested_reviewers: Option<Vec<Reviewer>>,
    /// What den did not read to filter them
    /// ([`Decision::suggested_reviewers_unread`]).
    suggested_reviewers_unread: Option<String>,
}

impl ReviewFindings {
    /// The one place a PR's review policy is chosen: the Linear Workflow's
    /// review rules when its Owner has them, milestone 1's otherwise.
    fn assess(pr: &PullRequest, policy: Option<&ReviewPolicy>) -> Self {
        match policy {
            Some(policy) => workflow::assess(pr, policy),
            None => Self::milestone_1(pr),
        }
    }

    /// Milestone 1's review rules: GitHub's review decision.
    fn milestone_1(pr: &PullRequest) -> Self {
        let approval = approval(pr);
        let merge = match approval {
            Approval::NoReviewRequired => "no review required and no changes requested",
            _ => "approved",
        };
        Self {
            address_review: address_review(pr),
            approved: "approved".to_owned(),
            merge: merge.to_owned(),
            approval,
            request_review: None,
            merge_blockers: Vec::new(),
            notes: Vec::new(),
            suggested_reviewers: None,
            suggested_reviewers_unread: None,
        }
    }
}

fn decide(ctx: &Context<'_>, pr: &PullRequest) -> Decision {
    let item = WorkItem {
        id: pr.id(),
        identifier: None,
        title: pr.title.clone(),
        url: pr.url.clone(),
        updated_at: pr.updated_at,
    };
    let policy = ctx.policy(&pr.repo.owner);
    let ReviewFindings {
        approval,
        address_review,
        request_review,
        approved,
        merge: merge_review,
        merge_blockers: review_blockers,
        notes: review_notes,
        suggested_reviewers,
        suggested_reviewers_unread,
    } = ReviewFindings::assess(pr, policy.review.as_ref());
    // Every Decision below carries the suggested reviewers, whatever its step.
    let with_reviewers = move |mut decision: Decision| {
        decision.suggested_reviewers = suggested_reviewers;
        decision.suggested_reviewers_unread = suggested_reviewers_unread;
        decision
    };
    let merge_blockers = merge_blockers(pr, &approval, review_blockers);
    let mut rules = vec![
        (NextStep::FinishDraft, draft(pr)),
        (NextStep::ResolveConflicts, conflicts(pr)),
        (NextStep::FixCi, fix_ci(pr.details.as_ref())),
        (NextStep::AddressReview, address_review),
        (
            NextStep::Review,
            review(pr.details.as_ref(), ctx.viewers.github),
        ),
    ];
    rules.extend(request_review.map(|finding| (NextStep::RequestReview, finding)));
    rules.push((NextStep::Merge, merge(pr, &merge_review, &merge_blockers)));
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
    other_conditions.extend(notes(
        pr,
        &approval,
        &approved,
        &merge_blockers,
        winner_step,
    ));
    other_conditions.extend(review_notes);

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
        let decision = Decision::new(
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
        return with_reviewers(decision);
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
    let groups = ctx.correlation.for_pull_request(&pr.repo, pr.number).groups;
    if step == NextStep::Merge
        && let Some((held, why_held)) = held_back(policy, pr, &groups)
    {
        let mut decision = Decision::new(
            ctx,
            item,
            None,
            Attribution::nobody(format!("{}, so nobody owes a Next Step", held.label())),
            Explanation {
                no_step_reason: Some(format!(
                    "{}: {why_held}; it would otherwise Merge ({winning_rule})",
                    held.label()
                )),
                other_conditions,
                unknowns,
                ..Explanation::default()
            },
        );
        decision.held_back = Some(held);
        return with_reviewers(decision);
    }
    let membership = (step == NextStep::Merge).then(|| membership_unread(ctx, pr));
    if let Some((_, Some(unread))) = &membership {
        unknowns.push(unread.clone());
    }
    let mut decision = Decision::new(
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
    );
    if let Some((sources, _)) = membership {
        decision.coverage.extend(sources);
    }
    with_reviewers(decision)
}

/// The one place a PR that would Merge is held back instead (spec 0002:
/// "would Merge" is then filtered): in a Held Repo it is held for release;
/// otherwise an attached member of a Landing Group of its own Owner lands
/// with its Linear Issue (#37). A Pull Request attached to another Owner's
/// Linear Issue is no member of that group (a Landing Group's Pull Requests
/// are its Owner's; that issue reports it "not surveyed"). `None`: Merge.
fn held_back(
    policy: &LinearWorkflowPolicy,
    pr: &PullRequest,
    groups: &[GroupRef],
) -> Option<(HeldBack, String)> {
    if policy.holds(&pr.repo) {
        return Some((
            HeldBack::ForRelease,
            format!(
                "{} is a Held Repo (workflow policy: a Held Repo's PRs are held for release, not merged)",
                pr.repo
            ),
        ));
    }
    let identifiers: Vec<String> = groups
        .iter()
        .filter(|g| same_login(g.issue.owner(), &pr.repo.owner))
        .map(|g| g.identifier.clone())
        .collect();
    if identifiers.is_empty() {
        return None;
    }
    let why = format!(
        "it is attached in Linear to {} (workflow policy: a Landing Group's Pull Requests land together, through its Linear Issue's Land, never by themselves)",
        identifiers.join(", ")
    );
    Some((HeldBack::LandsWith { identifiers }, why))
}

/// For a PR that Merges in an Owner with a Linear Workflow: the Owner's
/// Linear Issue lists that den didn't read completely, or whose issues'
/// attachments it didn't read in full (they say whether it is a Landing
/// Group member), for its coverage; and what den couldn't establish. Nothing when
/// everything was read, and nothing for an Owner without a Linear Workflow,
/// which records no Linear source.
fn membership_unread(ctx: &Context<'_>, pr: &PullRequest) -> (Vec<CoverageEntry>, Option<String>) {
    let owner = &pr.repo.owner;
    let mut sources: Vec<CoverageEntry> = ctx
        .observation
        .coverage
        .entries()
        .iter()
        .filter(|e| match &e.source {
            Source::Owner {
                owner: o,
                kind:
                    SourceKind::Linear(
                        LinearSourceKind::AssignedIssues | LinearSourceKind::StartTeamIssues,
                    ),
            } => same_login(o, owner) && !e.state.is_complete(),
            _ => false,
        })
        .cloned()
        .collect();
    let mut unread: Vec<String> = sources
        .iter()
        .map(|e| format!("{} {}", e.source.label(), e.state.describe()))
        .collect();
    let partly_attached: Vec<&LinearIssue> = ctx
        .observation
        .linear
        .issues
        .iter()
        .filter(|issue| same_login(&issue.owner, owner))
        .filter(|issue| matches!(issue.attachments.read, AttachmentsRead::Incomplete { .. }))
        .collect();
    unread.extend(
        partly_attached
            .iter()
            .map(|issue| format!("{}'s attachments weren't all read", issue.identifier)),
    );
    unread.dedup();
    // An issue's attachments are read with the list that listed it: that
    // list's follow-up is what den didn't read in full, so it joins the
    // coverage too, and the row reads incomplete (#35's gap b).
    for entry in attachments_unread(&partly_attached) {
        match sources.iter_mut().find(|e| e.source.same(&entry.source)) {
            Some(existing) => {
                let earlier = std::mem::replace(&mut existing.state, SourceState::Complete);
                existing.state = earlier.worst(entry.state);
            }
            None => sources.push(entry),
        }
    }
    let unknown = (!unread.is_empty()).then(|| {
        format!(
            "whether it lands with a Linear Issue (a Landing Group member never merges by itself): {}",
            unread.join("; ")
        )
    });
    (sources, unknown)
}

/// One partial entry per Linear source that listed `issues`, each naming
/// the issues whose attachments den didn't read in full, and why.
fn attachments_unread(issues: &[&LinearIssue]) -> Vec<CoverageEntry> {
    let mut entries: Vec<(Source, Vec<String>)> = Vec::new();
    for issue in issues {
        let AttachmentsRead::Incomplete { why } = &issue.attachments.read else {
            continue;
        };
        let source = Source::Owner {
            owner: issue.owner.clone(),
            kind: SourceKind::Linear(issue.listed_by),
        };
        let what = format!("{} ({why})", issue.identifier);
        match entries.iter_mut().find(|(s, _)| s.same(&source)) {
            Some((_, whats)) => whats.push(what),
            None => entries.push((source, vec![what])),
        }
    }
    entries
        .into_iter()
        .map(|(source, whats)| CoverageEntry {
            source,
            state: SourceState::Partial(Incompleteness::FollowUpTruncated {
                what: format!("the attachments of {}", whats.join(", ")),
            }),
        })
        .collect()
}

fn waiting_on(step: NextStep, pr: &PullRequest, ctx: &Context<'_>) -> Attribution {
    let viewer = ctx.viewers.github;
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
    reasons.extend(unresolved_threads(details));
    if !reasons.is_empty() {
        return Finding::Holds(reasons.join(", "));
    }
    if !details.threads_complete {
        return Finding::Unknown("not every review thread was read".to_owned());
    }
    Finding::No
}

/// "2 unresolved review threads" ("at least" when not every thread was read).
fn unresolved_threads(details: &PrDetails) -> Option<String> {
    if details.unresolved_threads == 0 {
        return None;
    }
    let at_least = if details.threads_complete {
        ""
    } else {
        "at least "
    };
    Some(format!(
        "{at_least}{} unresolved review {}",
        details.unresolved_threads,
        plural(details.unresolved_threads as usize, "thread")
    ))
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
            Reviewer::Team(team) if viewer.belongs_to(team) => format!("your {}", reviewer.label()),
            other => other.label(),
        })
        .collect();
    Finding::Holds(format!("review requested from {}", names.join(", ")))
}

/// `review` is the review half of the rule ("approved").
fn merge(pr: &PullRequest, review: &str, blockers: &[String]) -> Finding {
    if !blockers.is_empty() {
        return Finding::No;
    }
    Finding::Holds(format!(
        "{review}, checks passing and mergeable into {}",
        pr.base_ref
    ))
}

/// Everything that stops Merge. Empty means Merge holds: approved (or no
/// review required), no unresolved review threads and all threads read, every
/// review request read, every check (required or advisory) passing and all
/// read, known mergeable, not a draft, not in a merge queue, and GitHub's own
/// merge state agreeing. `review_blockers` are what the review rules add
/// (under a Linear Workflow, someone awaiting re-review, say).
fn merge_blockers(
    pr: &PullRequest,
    approval: &Approval,
    review_blockers: Vec<String>,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if pr.is_draft {
        blockers.push("the PR is a draft".to_owned());
    }
    match approval {
        Approval::Approved | Approval::NoReviewRequired => {}
        Approval::NotApproved(why) => blockers.push(why.clone()),
        Approval::Unknown => blockers.push("approval could not be established".to_owned()),
    }
    blockers.extend(review_blockers);
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
/// approval (as `approved` reads). Skipped where the winning rule already
/// says it.
fn notes(
    pr: &PullRequest,
    approval: &Approval,
    approved: &str,
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
        notes.push(approved.to_owned());
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
        ReviewDecision::ChangesRequested => Approval::NotApproved("changes requested".to_owned()),
        ReviewDecision::ReviewRequired => {
            Approval::NotApproved("not approved: a review is required".to_owned())
        }
        // Without GitHub's decision, a latest review asking for changes still
        // counts; nothing else can establish approval.
        ReviewDecision::Unknown if latest_review_requests_changes(pr) => {
            Approval::NotApproved("changes requested".to_owned())
        }
        ReviewDecision::Unknown => Approval::Unknown,
        ReviewDecision::NotRequired => {
            let Some(details) = &pr.details else {
                return Approval::Unknown;
            };
            let reviews = &details.latest_reviews;
            if latest_review_requests_changes(pr) {
                Approval::NotApproved("changes requested".to_owned())
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
