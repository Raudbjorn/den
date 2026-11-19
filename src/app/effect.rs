//! Effects: what the App asks the ports to do. The App never does I/O itself.

use super::event::SurveyId;
use crate::ports::SurveyRequest;

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
