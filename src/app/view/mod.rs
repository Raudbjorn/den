//! Building the ViewModel from the App's state, one module per section.

mod detail;
mod footer;
mod format;
mod header;
mod list;
mod scope;
mod sidebar;

use super::scope::ScopeState;
use super::view_model::{Overlay, ViewModel};
use crate::{
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
    pub overlay: Option<&'a Overlay>,
}

pub fn build(state: &State<'_>) -> ViewModel {
    ViewModel {
        header: header::build(state),
        sidebar: sidebar::build(state),
        list: list::build(state),
        detail: detail::build(state),
        overlay: state.overlay.cloned(),
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
