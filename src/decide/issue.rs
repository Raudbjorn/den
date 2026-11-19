//! Issue rules. Owned by the Issues ticket (#5); not implemented yet.
//!
//! The Observation's `issues` slot is empty until #5 fetches it, so this
//! produces nothing in practice. Should facts arrive first, each item is kept
//! visible with no Next Step rather than dropped.

use super::{Attribution, Context, Decision, Explanation, WorkItem};

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .issues
        .iter()
        .map(|issue| {
            let item = WorkItem {
                id: issue.id(),
                title: issue.title.clone(),
                url: issue.url.clone(),
                updated_at: issue.updated_at,
            };
            Decision::new(
                ctx,
                item,
                None,
                Attribution::nobody("no Next Step"),
                Explanation {
                    unknowns: vec!["Issue rules are not implemented yet".to_owned()],
                    ..Explanation::default()
                },
            )
        })
        .collect()
}
