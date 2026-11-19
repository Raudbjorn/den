//! Events: everything that can happen to the App.

use crate::{domain::Observation, ports::SurveyFailure};
use jiff::Timestamp;

/// Identifies one survey, so a late answer to an older one is recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurveyId(pub u64);

/// A key, already translated from the terminal's encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Esc,
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
}
