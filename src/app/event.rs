//! Events: everything that can happen to the App.

use crate::{
    cache::{CacheError, CachedObservation},
    domain::Observation,
    handoff::{JobsCheck, LaunchReport, WorkspaceView},
    ports::SurveyFailure,
    upstreams::UpstreamStoreError,
};
use jiff::Timestamp;

/// Identifies one survey, so a late answer to an older one is recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurveyId(pub u64);

/// Identifies one launch sheet, so a late answer for a closed sheet is recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LaunchId(pub u64);

/// A key, already translated from the terminal's encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Tab,
    Enter,
    Esc,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Char(char),
    /// Ctrl-C.
    Interrupt,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// den started; the first event the App sees.
    Started {
        now: Timestamp,
    },
    Key(Key),
    /// The terminal's size, in cells: sent once at start (before `Started` is
    /// fine) and whenever it changes. Until then den assumes its design size.
    Resized {
        cols: u16,
        rows: u16,
    },
    /// Time passed (sent periodically so ages stay current).
    Tick {
        now: Timestamp,
    },
    /// A survey finished, with an Observation (boxed: it is large) or a failure.
    SurveyFinished {
        id: SurveyId,
        outcome: Result<Box<Observation>, SurveyFailure>,
        now: Timestamp,
    },
    /// The cache was read: the last viewer's Observation, none, or why not.
    CacheLoaded {
        outcome: Result<Option<Box<CachedObservation>>, CacheError>,
    },
    /// The cache was written, or why not.
    CacheStored {
        outcome: Result<(), CacheError>,
    },
    /// The workspace was inspected for a launch sheet.
    LaunchInspected {
        id: LaunchId,
        view: Box<WorkspaceView>,
    },
    /// A launch ran (or stopped); what it did.
    LaunchFinished {
        id: LaunchId,
        report: Box<LaunchReport>,
    },
    /// The recorded Jobs' worktrees were checked (#14), or why the Job records
    /// couldn't be read.
    JobsChecked {
        outcome: Result<Box<JobsCheck>, String>,
    },
    /// The Upstream choices were saved, or why not.
    UpstreamsStored {
        outcome: Result<(), UpstreamStoreError>,
    },
}
