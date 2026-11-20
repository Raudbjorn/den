//! Branch rules. Owned by the Branches ticket (#6); not implemented yet.
//!
//! The Observation's `branches` slot is empty until #6 fetches it, so this
//! produces nothing in practice. Should facts arrive first, each item is kept
//! visible with no Next Step rather than dropped.

use super::{Attribution, Context, Decision, Explanation, WorkItem};

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .branches
        .iter()
        .map(|branch| {
            let item = WorkItem {
                id: branch.id(),
                title: branch.name.clone(),
                url: branch.url.clone(),
                updated_at: branch.last_commit_at,
            };
            Decision::new(
                ctx,
                item,
                None,
                Attribution::nobody("no Next Step"),
                Explanation {
                    unknowns: vec!["Branch rules are not implemented yet".to_owned()],
                    ..Explanation::default()
                },
            )
        })
        .collect()
}
