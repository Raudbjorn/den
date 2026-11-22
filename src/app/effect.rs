//! Effects: what the App asks the ports to do. The App never does I/O itself.

use super::event::{LaunchId, SurveyId};
use crate::{
    cache::CachedObservation,
    domain::facts::LinearState,
    domain::{UpstreamTargets, WorkItemId},
    handoff::{InspectRequest, LaunchPlan, ReopenPlan},
    ports::{LinearIssueRequest, LinearSurveyRequest, MemberStatesRequest, SurveyRequest},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Survey GitHub; answered by `Event::SurveyFinished` with the same id.
    Survey {
        id: SurveyId,
        request: SurveyRequest,
    },
    /// Survey Linear for the Owners with a Linear Workflow (#32); answered by
    /// `Event::LinearSurveyFinished` with the same id as the GitHub survey it
    /// accompanies. Never emitted when no Owner has a Linear Workflow.
    SurveyLinear {
        id: SurveyId,
        request: LinearSurveyRequest,
    },
    /// Read the states of Landing Group members den didn't find among the
    /// open Pull Requests (#37); answered by `Event::MemberStatesRead` with
    /// the same id as the survey round it follows. Emitted once both of that
    /// round's surveys have answered, and only when some member needs it.
    ReadMemberStates {
        id: SurveyId,
        request: MemberStatesRequest,
    },
    /// Read the cache; answered by `Event::CacheLoaded`. Emitted before the
    /// startup survey, so cached data shows before any network call completes;
    /// a survey that answers first anyway is held until this one answers.
    LoadCache,
    /// Write what den shows to the cache; answered by `Event::CacheStored`.
    StoreCache { cached: Box<CachedObservation> },
    /// Look at the workspace for a launch sheet (reads only); answered by
    /// `Event::LaunchInspected` with the same id.
    InspectLaunch {
        id: LaunchId,
        request: Box<InspectRequest>,
    },
    /// Carry out a confirmed LaunchPlan; answered by `Event::LaunchFinished`.
    Launch { id: LaunchId, plan: Box<LaunchPlan> },
    /// Check every recorded Job's worktree (reads only, bounded; #14);
    /// answered by `Event::JobsChecked`.
    CheckJobs,
    /// Open a Live Job's window again, resuming its session (#14); answered by
    /// `Event::LaunchFinished`.
    Reopen { id: LaunchId, plan: Box<ReopenPlan> },
    /// Put `text` (a needs-decision Point's answering marker line, #77) on the
    /// clipboard for `item`; answered by `Event::Copied`. den posts nothing.
    CopyToClipboard { item: WorkItemId, text: String },
    /// Read a Linear Issue again just before writing to it (#38), for the
    /// sheet `id`; answered by `Event::LinearIssueRead`. Emitted only after
    /// `Enter` on a sheet that will write to Linear, and after a write whose
    /// outcome is unknown.
    ReadLinearIssue {
        id: LaunchId,
        request: Box<LinearIssueRequest>,
    },
    /// Assign a Linear Issue to the Linear viewer (ADR-0004), for the sheet
    /// `id`; answered by `Event::LinearWriteFinished`. Emitted only from a
    /// confirmed sheet, after a re-read showed the issue as the sheet did.
    AssignLinearIssue {
        id: LaunchId,
        request: Box<LinearIssueRequest>,
    },
    /// Move a Linear Issue to `state` (ADR-0004), for the sheet `id`;
    /// answered by `Event::LinearWriteFinished`. Emitted only from a
    /// confirmed sheet, after a re-read and any assignment it asked for.
    MoveLinearIssue {
        id: LaunchId,
        request: Box<LinearIssueRequest>,
        state: Box<LinearState>,
    },
    /// Save which forks target their Upstream (#13); answered by
    /// `Event::UpstreamsStored`.
    StoreUpstreams { upstreams: UpstreamTargets },
    /// Leave den.
    Quit,
    /// Write to den's structured log.
    Log(LogRecord),
}

impl Effect {
    pub fn log(level: LogLevel, message: impl Into<String>) -> Self {
        Self::Log(LogRecord {
            level,
            message: message.into(),
        })
    }
}
