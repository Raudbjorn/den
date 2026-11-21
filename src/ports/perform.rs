//! Carrying out one Effect through the ports. Shared by `main`'s background
//! worker and the seam-2 test harness, so tests exercise the same wiring.

use super::{LinearWriteKind, Ports};
use crate::{
    app::{Effect, Event},
    handoff,
};
use std::time::Duration;

/// Performs `effect` and returns the events it produced (none for effects the
/// caller handles itself, such as Quit and Log).
pub fn perform(effect: &Effect, ports: &Ports) -> Vec<Event> {
    match effect {
        Effect::Survey { id, request } => {
            let outcome = ports.github.survey(request).map(Box::new);
            vec![Event::SurveyFinished {
                id: *id,
                outcome,
                now: ports.clock.now(),
            }]
        }
        Effect::ReadMemberStates { id, request } => vec![Event::MemberStatesRead {
            id: *id,
            read: Box::new(ports.github.member_states(request)),
            now: ports.clock.now(),
        }],
        Effect::SurveyLinear { id, request } => vec![Event::LinearSurveyFinished {
            id: *id,
            survey: Box::new(ports.linear.survey(request)),
            now: ports.clock.now(),
        }],
        Effect::LoadCache => vec![Event::CacheLoaded {
            outcome: ports.cache.load().map(|c| c.map(Box::new)),
        }],
        Effect::StoreCache { cached } => vec![Event::CacheStored {
            outcome: ports.cache.store(cached),
        }],
        Effect::InspectLaunch { id, request } => vec![Event::LaunchInspected {
            id: *id,
            view: Box::new(handoff::inspect(
                request,
                ports.workspace.as_ref(),
                ports.launcher.as_ref(),
                ports.jobs.as_ref(),
            )),
        }],
        Effect::Launch { id, plan } => vec![Event::LaunchFinished {
            id: *id,
            report: Box::new(handoff::execute(
                plan,
                ports.workspace.as_ref(),
                ports.launcher.as_ref(),
                ports.jobs.as_ref(),
                ports.clock.now(),
            )),
        }],
        Effect::CheckJobs => vec![Event::JobsChecked {
            outcome: ports
                .jobs
                .load()
                .map(|jobs| {
                    Box::new(handoff::live::check_jobs(
                        &jobs,
                        ports.workspace.as_ref(),
                        Duration::from_secs(handoff::live::JOB_CHECK_BUDGET_SECS),
                    ))
                })
                .map_err(|e| e.to_string()),
        }],
        Effect::Reopen { id, plan } => vec![Event::LaunchFinished {
            id: *id,
            report: Box::new(handoff::live::reopen(
                plan,
                ports.workspace.as_ref(),
                ports.launcher.as_ref(),
            )),
        }],
        Effect::ReadLinearIssue { id, request } => vec![Event::LinearIssueRead {
            id: *id,
            read: Box::new(ports.linear.read_issue(request)),
        }],
        Effect::AssignLinearIssue { id, request } => vec![Event::LinearWriteFinished {
            id: *id,
            write: LinearWriteKind::AssignToViewer,
            outcome: Box::new(ports.linear_writer.assign_to_viewer(request)),
        }],
        Effect::MoveLinearIssue { id, request, state } => vec![Event::LinearWriteFinished {
            id: *id,
            write: LinearWriteKind::MoveState,
            outcome: Box::new(ports.linear_writer.move_state(request, state)),
        }],
        Effect::StoreUpstreams { upstreams } => vec![Event::UpstreamsStored {
            outcome: ports.upstreams.store(upstreams),
        }],
        Effect::Quit | Effect::Log(_) => Vec::new(),
    }
}
