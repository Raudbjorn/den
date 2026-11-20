//! Recorded Jobs in the list and the detail (#14): which Work Items have a
//! Live Job, a gone worktree, or one den can't tell about.

use crate::{
    app::view_model::{JobDetail, JobMarker},
    domain::WorkItemId,
    handoff::{JobCheck, WorktreeState, live},
};

pub fn marker(state: &WorktreeState) -> JobMarker {
    match state {
        WorktreeState::Present => JobMarker::Live,
        WorktreeState::Missing => JobMarker::Missing,
        WorktreeState::Unknown(_) => JobMarker::Unknown,
    }
}

/// The detail's Job section: the check that says most about `item`'s Jobs.
pub fn detail(checks: &[JobCheck], item: &WorkItemId) -> Option<JobDetail> {
    let check = live::most_telling(checks, item)?;
    let job = &check.job;
    let summary = match &check.state {
        WorktreeState::Present => {
            "Live Job: its worktree exists and git has it registered; c asks open existing or start fresh"
                .to_owned()
        }
        WorktreeState::Missing => "The Job's worktree is gone; c launches anew".to_owned(),
        WorktreeState::Unknown(why) => format!(
            "The Job's worktree is unknown: {why}. den doesn't treat it as gone and deletes nothing"
        ),
    };
    Some(JobDetail {
        marker: marker(&check.state),
        summary,
        lines: vec![
            job.worktree.display().to_string(),
            format!(
                "session {}, launched {}",
                job.session,
                job.launched_at.strftime("%Y-%m-%d %H:%M UTC")
            ),
            format!("branch {}", job.branch),
        ],
    })
}
