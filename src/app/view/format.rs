//! Small text formatting shared by the sections.

use jiff::{SignedDuration, Timestamp};

const SECS_PER_MINUTE: i64 = 60;
const SECS_PER_HOUR: i64 = 60 * SECS_PER_MINUTE;
const SECS_PER_DAY: i64 = 24 * SECS_PER_HOUR;

/// A compact age: "now", "5m", "3h", "12d".
pub fn age(now: Option<Timestamp>, then: Timestamp) -> String {
    let Some(now) = now else {
        return String::new();
    };
    let secs = now.duration_since(then).as_secs().max(0);
    match secs {
        s if s < SECS_PER_MINUTE => "now".to_owned(),
        s if s < SECS_PER_HOUR => format!("{}m", s / SECS_PER_MINUTE),
        s if s < SECS_PER_DAY => format!("{}h", s / SECS_PER_HOUR),
        s => format!("{}d", s / SECS_PER_DAY),
    }
}

/// "just now", "4 min ago", "2 h ago", "3 days ago".
pub fn ago(now: Option<Timestamp>, then: Timestamp) -> String {
    let Some(now) = now else {
        return String::new();
    };
    let elapsed: SignedDuration = now.duration_since(then);
    match elapsed.as_secs().max(0) {
        s if s < SECS_PER_MINUTE => "just now".to_owned(),
        s if s < SECS_PER_HOUR => format!("{} min ago", s / SECS_PER_MINUTE),
        s if s < SECS_PER_DAY => format!("{} h ago", s / SECS_PER_HOUR),
        s => format!("{} days ago", s / SECS_PER_DAY),
    }
}

/// "23:01 UTC" (den shows UTC; the user's zone is a later nicety).
pub fn clock_time(at: Timestamp) -> String {
    at.strftime("%H:%M UTC").to_string()
}
