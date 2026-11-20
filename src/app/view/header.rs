//! The header: Scope, survey status and freshness, coverage summary.

use super::{State, format, scope};
use crate::{
    app::view_model::Header,
    domain::{Coverage, SourceState},
};

pub fn build(state: &State<'_>) -> Header {
    let (scope, scopes) = scope::describe(state);
    let status = status(state);
    let (coverage, coverage_complete) = match state.observation {
        Some(observation) => (
            summarise(&observation.coverage),
            observation.coverage.is_complete(),
        ),
        None => ("Not surveyed yet".to_owned(), false),
    };
    Header {
        scope,
        scopes,
        status,
        coverage,
        coverage_complete,
    }
}

fn status(state: &State<'_>) -> String {
    if state.config.is_err() {
        return "No usable config".to_owned();
    }
    let refreshing = if state.surveying {
        " · surveying…"
    } else {
        ""
    };
    match (state.observation, state.failure) {
        (Some(observation), failure) => {
            let observed = format!(
                "observed {}, {}",
                format::clock_time(observation.observed_at),
                format::ago(state.now, observation.observed_at)
            );
            match failure {
                Some(failure) => {
                    format!("{observed} · last refresh failed: {}", failure.headline())
                }
                None => format!("{observed}{refreshing}"),
            }
        }
        (None, Some(failure)) => failure.headline().to_owned(),
        (None, None) if state.surveying => "Surveying…".to_owned(),
        (None, None) => "Not surveyed yet".to_owned(),
    }
}

/// "Complete", or what is missing: failed and partial sources by name, and the
/// kinds den has not fetched.
pub fn summarise(coverage: &Coverage) -> String {
    if coverage.is_complete() {
        return "Complete".to_owned();
    }
    let mut troubled = Vec::new();
    let mut not_fetched = Vec::new();
    for entry in coverage.incomplete() {
        match &entry.state {
            SourceState::NotYetFetched => {
                let label = match &entry.source {
                    crate::domain::Source::Owner { kind, .. }
                    | crate::domain::Source::Repo { kind, .. } => kind.label().to_owned(),
                    crate::domain::Source::Viewer => entry.source.label(),
                };
                if !not_fetched.contains(&label) {
                    not_fetched.push(label);
                }
            }
            state => troubled.push(format!("{} {}", entry.source.label(), state.describe())),
        }
    }
    let mut parts = Vec::new();
    if !troubled.is_empty() {
        parts.push(format!("Incomplete: {}", troubled.join("; ")));
    }
    if !not_fetched.is_empty() {
        parts.push(format!("not fetched yet: {}", not_fetched.join(", ")));
    }
    parts.join(" · ")
}
