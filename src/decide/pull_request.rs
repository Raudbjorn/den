//! Pull Request rules. Owned by the PR rules ticket (#4).
//!
//! Implemented so far: draft → Finish draft (Waiting On the PR author). Every
//! other PR has no Next Step, and its Explanation says the other rules are not
//! evaluated yet.

use super::{Attribution, Context, Decision, Explanation, WorkItem};
use crate::domain::{NextStep, facts::PullRequest};

const AUTHOR_ROLE: &str = "the PR author";
const RULES_NOT_EVALUATED: &str = "conflicts, checks, reviews and mergeability are not read yet, so later PR rules were not evaluated";

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .pull_requests
        .iter()
        .map(|pr| decide(ctx, pr))
        .collect()
}

fn decide(ctx: &Context<'_>, pr: &PullRequest) -> Decision {
    let item = WorkItem {
        id: pr.id(),
        title: pr.title.clone(),
        url: pr.url.clone(),
        updated_at: pr.updated_at,
    };
    let unknowns = vec![RULES_NOT_EVALUATED.to_owned()];
    if pr.is_draft {
        return Decision::new(
            ctx,
            item,
            Some(NextStep::FinishDraft),
            Attribution::person(ctx.viewer, pr.author.as_deref(), AUTHOR_ROLE),
            Explanation {
                winning_rule: Some("the PR is a draft".to_owned()),
                unknowns,
                ..Explanation::default()
            },
        );
    }
    Decision::new(
        ctx,
        item,
        None,
        Attribution::nobody("no Next Step"),
        Explanation {
            no_step_reason: Some("not a draft; no other PR rule is evaluated yet".to_owned()),
            unknowns,
            ..Explanation::default()
        },
    )
}
