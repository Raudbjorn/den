//! The header: Scope, survey status and freshness, coverage summary.

use super::{State, coverage, format, scope};
use crate::{
    app::view_model::Header,
    cache,
    domain::{Source, SourceState},
    ports::SurveyFailure,
};

pub fn build(state: &State<'_>) -> Header {
    let (scope, scopes) = scope::describe(state);
    let status = status(state);
    let (coverage, coverage_complete) = match state.observation {
        Some(observation) => (summarise(state), cache::survey_complete(observation)),
        None => ("Not surveyed yet".to_owned(), false),
    };
    Header {
        scope,
        scopes,
        status,
        coverage,
        coverage_complete,
        repos: coverage::by_repo(state),
    }
}

/// When the shown data was observed and how old it is, whether it is cached,
/// whether the last refresh failed, rate limits, and whether a refresh is running.
fn status(state: &State<'_>) -> String {
    if state.config.is_err() {
        return "No usable config".to_owned();
    }
    let Some(observation) = state.observation else {
        return match state.failure {
            Some(failure) => failure_text(failure),
            None if state.surveying => "Surveying…".to_owned(),
            None => "Not surveyed yet".to_owned(),
        };
    };
    let mut parts = vec![format!(
        "{}observed {}, {}",
        if state.from_cache { "cached: " } else { "" },
        format::clock_time(observation.observed_at),
        format::ago(state.now, observation.observed_at)
    )];
    if let Some(failure) = state.failure {
        parts.push(format!("last refresh failed: {}", failure_text(failure)));
    }
    if let Some(note) = coverage::rate_limit_note(observation) {
        parts.push(note);
    }
    if state.surveying {
        parts.push("refreshing…".to_owned());
    }
    parts.join(" · ")
}

/// The failure's headline; a rate limit also says when it resets.
pub fn failure_text(failure: &SurveyFailure) -> String {
    match failure {
        SurveyFailure::RateLimited { .. } => failure.to_string(),
        other => other.headline().to_owned(),
    }
}

/// "Complete", or what is missing: failed and partial sources by name (with
/// the age of any data kept for them), and the kinds den has not fetched.
pub fn summarise(state: &State<'_>) -> String {
    let Some(observation) = state.observation else {
        return "Not surveyed yet".to_owned();
    };
    if cache::survey_complete(observation) {
        return "Complete".to_owned();
    }
    let mut troubled = Vec::new();
    let mut not_fetched = Vec::new();
    for entry in observation.coverage.incomplete() {
        match &entry.state {
            SourceState::NotYetFetched => {
                let label = match &entry.source {
                    Source::Owner { kind, .. } | Source::Repo { kind, .. } => {
                        kind.label().to_owned()
                    }
                    Source::Viewer => entry.source.label(),
                };
                if !not_fetched.contains(&label) {
                    not_fetched.push(label);
                }
            }
            state_of_entry => {
                let line = format!("{} {}", entry.source.label(), state_of_entry.describe());
                troubled.push(match coverage::carried_note(state, &entry.source) {
                    Some(note) => format!("{line} ({note})"),
                    None => line,
                });
            }
        }
    }
    let mut parts = Vec::new();
    if !troubled.is_empty() {
        parts.push(format!("Incomplete: {}", troubled.join("; ")));
    }
    if !not_fetched.is_empty() {
        parts.push(format!("not fetched yet: {}", not_fetched.join(", ")));
    }
    if parts.is_empty() {
        // Every recorded source was complete, but not every Owner was read.
        parts.push("Incomplete: not every watched Owner has been surveyed".to_owned());
    }
    parts.join(" · ")
}
