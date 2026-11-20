//! Branch rules (spec, "Rule table" and "Waiting On rules").
//!
//! A Branch is a Work Item only when it is not its Repo's default branch, was
//! never the head of any PR (open, merged or closed), and has commits that are
//! not on its comparison branch. Then: last commit within
//! [`STALE_BRANCH_AGE_DAYS`] → Open PR; older (a Stale Branch) → Prune or
//! revive. Both are Waiting On the author of the last commit; an author GitHub
//! could not match to a login leaves the item Waiting On nobody, unattributed.
//!
//! The comparison branch is the Repo's own default branch, or, for a fork whose
//! target is its Upstream (#13), the Upstream's. A Branch whose commits were
//! counted against anything else (counted before the target was switched, or
//! kept from an earlier survey) gets no Next Step until a survey compares it
//! again: den never judges it against a branch the user didn't choose.

use super::{Attribution, Context, Decision, Explanation, WorkItem};
use crate::domain::{
    Comparison, NextStep, comparison_for,
    facts::{Branch, ComparisonBranch},
};
use jiff::{SignedDuration, Timestamp};

/// A Branch whose last commit is more than this many days old is a Stale Branch.
pub const STALE_BRANCH_AGE_DAYS: i64 = 30;
const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

const AUTHOR_ROLE: &str = "the author of the Branch's last commit";
const NOT_PROOF: &str = "this is a decision, not proof the work is unshipped: squash and rebase merges leave Branches like this behind";

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .branches
        .iter()
        .filter(|branch| is_candidate(ctx, branch))
        .map(|branch| decide(ctx, branch))
        .collect()
}

/// False for what is never a Work Item: the default branch, a Branch that was
/// ever a PR head, and one with nothing that is not on its comparison branch.
fn is_candidate(ctx: &Context<'_>, branch: &Branch) -> bool {
    let is_default = ctx
        .observation
        .repo(&branch.repo)
        .and_then(|repo| repo.default_branch.as_deref())
        .is_some_and(|default| default == branch.name);
    let is_its_own_comparison =
        branch.comparison.repo == branch.repo && branch.comparison.name == branch.name;
    !is_default && !is_its_own_comparison && !branch.ever_had_pr && branch.commits_ahead != Some(0)
}

fn decide(ctx: &Context<'_>, branch: &Branch) -> Decision {
    let item = WorkItem {
        id: branch.id(),
        identifier: None,
        title: if branch.last_commit_headline.is_empty() {
            branch.name.clone()
        } else {
            branch.last_commit_headline.clone()
        },
        url: branch.url.clone(),
        updated_at: branch.last_commit_at,
    };
    let base = branch.comparison.describe(&branch.repo);
    if let Some(reason) = stale_comparison(ctx, branch) {
        return Decision::new(
            ctx,
            item,
            None,
            Attribution::nobody("no Next Step"),
            Explanation {
                no_step_reason: Some(reason),
                unknowns: vec![format!(
                    "commits ahead of the comparison branch it should have (it was compared against {base})"
                )],
                ..Explanation::default()
            },
        );
    }
    let Some(ahead) = branch.commits_ahead else {
        return Decision::new(
            ctx,
            item,
            None,
            Attribution::nobody("no Next Step"),
            Explanation {
                no_step_reason: Some(format!(
                    "den could not establish whether it has commits not on {base}"
                )),
                unknowns: vec![format!("commits ahead of {base}")],
                ..Explanation::default()
            },
        );
    };
    let age = age(ctx.now, branch.last_commit_at);
    let days = age.as_secs() / SECONDS_PER_DAY;
    let evidence = format!(
        "{ahead} commit{} not on {base}, never the head of a PR, last commit {days} day{} ago",
        plural(i64::from(ahead)),
        plural(days),
    );
    let stale = age > SignedDuration::from_secs(STALE_BRANCH_AGE_DAYS * SECONDS_PER_DAY);
    let (step, winning_rule, other_conditions) = if stale {
        (
            NextStep::PruneOrRevive,
            format!(
                "a Stale Branch ({evidence}, more than {STALE_BRANCH_AGE_DAYS}): decide whether to prune it or revive it"
            ),
            vec![NOT_PROOF.to_owned()],
        )
    } else {
        (
            NextStep::OpenPr,
            format!("unproposed work ({evidence}, within {STALE_BRANCH_AGE_DAYS})"),
            Vec::new(),
        )
    };
    let attribution = Attribution::person(
        &ctx.viewers.github,
        branch.last_commit_author.as_deref(),
        AUTHOR_ROLE,
    );
    let unknowns = if attribution.unattributed {
        vec![
            "the last commit's author is not matched to a GitHub login, so who owes this is unattributed"
                .to_owned(),
        ]
    } else {
        Vec::new()
    };
    Decision::new(
        ctx,
        item,
        Some(step),
        attribution,
        Explanation {
            winning_rule: Some(winning_rule),
            no_step_reason: None,
            other_conditions,
            unknowns,
        },
    )
}

/// Why `branch`'s comparison is not the one its Repo's target calls for, if it
/// isn't. A Repo den knows nothing about is taken as compared correctly.
fn stale_comparison(ctx: &Context<'_>, branch: &Branch) -> Option<String> {
    let repo = ctx.observation.repo(&branch.repo)?;
    let compared = branch.comparison.describe(&branch.repo);
    let expected: ComparisonBranch = match comparison_for(repo, ctx.upstreams) {
        Comparison::Against(expected) => expected,
        Comparison::Nothing => return None,
        Comparison::UpstreamUnavailable { fork } => {
            return Some(format!(
                "{fork}'s target is its Upstream, but den can't see the Upstream's default branch to compare against (its commits were counted against {compared})"
            ));
        }
    };
    if expected == branch.comparison {
        return None;
    }
    let target = if expected.repo == branch.repo {
        "its own default branch"
    } else {
        "its Upstream's default branch"
    };
    Some(format!(
        "its commits were counted against {compared}, but {} now compares against {target} {}; refresh (r) to compare again",
        branch.repo,
        expected.describe(&branch.repo)
    ))
}

/// How old the last commit is; a commit dated in the future counts as new.
fn age(now: Timestamp, last_commit_at: Timestamp) -> SignedDuration {
    now.duration_since(last_commit_at).max(SignedDuration::ZERO)
}

fn plural(count: i64) -> &'static str {
    if count == 1 { "" } else { "s" }
}
