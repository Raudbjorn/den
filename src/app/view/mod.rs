//! Building the ViewModel from the App's state, one module per section.

pub mod coverage;
mod detail;
mod footer;
mod format;
mod header;
mod list;
mod sidebar;

use super::view_model::ViewModel;
use crate::{
    cache::Carried,
    config::{Config, ConfigError},
    decide::Decision,
    domain::Observation,
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
    pub decisions: &'a [Decision],
    pub failure: Option<&'a SurveyFailure>,
    pub selected: Option<usize>,
    pub now: Option<Timestamp>,
}

pub fn build(state: &State<'_>) -> ViewModel {
    ViewModel {
        header: header::build(state),
        sidebar: sidebar::build(state),
        list: list::build(state),
        detail: detail::build(state),
        overlay: None,
        footer: footer::build(state),
    }
}
