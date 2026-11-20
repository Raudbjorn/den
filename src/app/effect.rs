//! Effects: what the App asks the ports to do. The App never does I/O itself.

use super::event::{LaunchId, SurveyId};
use crate::{
    cache::CachedObservation,
    domain::UpstreamTargets,
    handoff::{InspectRequest, LaunchPlan, ReopenPlan},
    ports::{LinearSurveyRequest, SurveyRequest},
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
    /// Read the cache; answered by `Event::CacheLoaded`. Emitted before the
    /// startup survey, so cached data shows before any network call completes.
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
