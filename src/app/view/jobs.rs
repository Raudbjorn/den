//! Recorded Jobs in the list and the detail (#14): which Work Items have a
//! Live Job, a gone worktree, or one den can't tell about.

use crate::{
    app::view_model::{JobActivity, JobActivityLine, JobDetail, JobMarker},
    domain::WorkItemId,
    handoff::{JobCheck, SessionActivity, WorktreeState, live},
};

/// A Live Job's session activity, as the list draws it (#80).
pub fn activity(activity: &SessionActivity) -> JobActivity {
    match activity {
        SessionActivity::Running => JobActivity::Running,
        SessionActivity::Idle => JobActivity::Idle,
        SessionActivity::Unknown(_) => JobActivity::Unknown,
    }
}

/// The detail's line on a Live Job's session (#80): what den saw, and that
/// it decides nothing.
fn activity_line(check: &JobCheck) -> Option<JobActivityLine> {
    let session = &check.job.session;
    let found = check.activity.as_ref()?;
    let text = match found {
        SessionActivity::Running => {
            format!("Session running: a local claude process has session {session}")
        }
        SessionActivity::Idle => format!(
            "Session idle: no local claude process has session {session}; it finished or stopped"
        ),
        SessionActivity::Unknown(why) => format!("Running or idle: unknown, {why}"),
    };
    Some(JobActivityLine {
        activity: activity(found),
        text,
    })
}

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
    // A Land Job covering a member names its Landing Group (#40).
    let land_job = job
        .subject
        .landing_group()
        .map(|group| format!("Land Job of {}: {}", group.identifier, group.members_text()));
    Some(JobDetail {
        marker: marker(&check.state),
        summary,
        activity: activity_line(check),
        lines: land_job
            .into_iter()
            .chain([
                job.worktree.display().to_string(),
                format!(
                    "session {}, launched {}",
                    job.session,
                    job.launched_at.strftime("%Y-%m-%d %H:%M UTC")
                ),
                format!("branch {}", job.branch),
            ])
            .collect(),
    })
}
