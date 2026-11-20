//! Building the ViewModel from the App's state, one module per section.

pub mod coverage;
mod detail;
mod footer;
mod format;
mod header;
mod launch;
mod list;
mod scope;
mod sidebar;

use super::OverlayState;
use super::scope::ScopeState;
use super::view_model::{Overlay, ViewModel};
use crate::{
    cache::Carried,
    config::{Config, ConfigError},
    decide::Decision,
    domain::Observation,
    layout::Layout,
    ordering::Scope,
    ports::SurveyFailure,
};
use jiff::Timestamp;

/// A read-only view of the App's state.
pub struct State<'a> {
    pub config: &'a Result<Config, ConfigError>,
    pub surveying: bool,
    pub observation: Option<&'a Observation>,
    /// Sources whose shown data is older than `observation` (kept from an
    /// earlier survey because the latest could not read them).
    pub carried: &'a [Carried],
    /// `observation` is the cached one; no survey has answered yet.
    pub from_cache: bool,
    /// The Decisions in Scope, ordered: what the list shows.
    pub decisions: &'a [Decision],
    /// Every Decision, ordered (for the sidebar's counts).
    pub all_decisions: &'a [Decision],
    pub owners: &'a [String],
    pub scope: &'a ScopeState,
    /// The sidebar's rows, as Scopes.
    pub sidebar: Vec<Scope>,
    pub failure: Option<&'a SurveyFailure>,
    pub selected: Option<usize>,
    pub now: Option<Timestamp>,
    pub layout: Layout,
    pub overlay: Option<&'a OverlayState>,
}

pub fn build(state: &State<'_>) -> ViewModel {
    ViewModel {
        header: header::build(state),
        sidebar: sidebar::build(state),
        list: list::build(state),
        detail: detail::build(state),
        overlay: state.overlay.map(|overlay| match overlay {
            OverlayState::DetailSheet => Overlay::DetailSheet,
            OverlayState::Launch(sheet) => Overlay::LaunchSheet(Box::new(launch::build(sheet))),
        }),
        footer: footer::build(state),
        layout: state.layout,
        splash: splash(state),
    }
}

/// The splash stands in for the plates only while den has nothing else to show:
/// the first survey is running and there is no Observation and no failure yet.
fn splash(state: &State<'_>) -> bool {
    state.config.is_ok()
        && state.surveying
        && state.observation.is_none()
        && state.failure.is_none()
}
