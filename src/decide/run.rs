//! Run rules. Owned by the Runs ticket (#7); not implemented yet.
//!
//! The Observation's `runs` slot is empty until #7 fetches it, so this
//! produces nothing in practice. Should facts arrive first, each item is kept
//! visible with no Next Step rather than dropped.

use super::{Attribution, Context, Decision, Explanation, WorkItem};

pub fn decide_all(ctx: &Context<'_>) -> Vec<Decision> {
    ctx.observation
        .runs
        .iter()
        .map(|run| {
            let item = WorkItem {
                id: run.id(),
                title: run.workflow_name.clone(),
                url: run.url.clone(),
                updated_at: run.updated_at,
            };
            Decision::new(
                ctx,
                item,
                None,
                Attribution::nobody("no Next Step"),
                Explanation {
                    unknowns: vec!["Run rules are not implemented yet".to_owned()],
                    ..Explanation::default()
                },
            )
        })
        .collect()
}
