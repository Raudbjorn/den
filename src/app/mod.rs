//! The App (test seam 2): `handle(Event) -> (ViewModel, Effects)`.
//!
//! The App holds den's state and decides what to show and what to ask the ports
//! for. It does no I/O and reads no clock: time arrives in events.

mod effect;
mod event;
mod view;
pub mod view_model;

pub use effect::{Effect, LogLevel, LogRecord};
pub use event::{Event, Key, SurveyId};
pub use view_model::ViewModel;

use crate::{
    cache::{self, CacheError, CachedObservation},
    config::{Config, ConfigError},
    decide::{self, Decision},
    domain::{Observation, SourceState, WorkItemId},
    ordering,
    ports::{SurveyFailure, SurveyRequest},
};
use jiff::Timestamp;

pub struct App {
    config: Result<Config, ConfigError>,
    /// The survey in flight, if any.
    running: Option<SurveyId>,
    next_survey: u64,
    /// What den shows: the latest survey merged with data kept for sources it
    /// could not read, or the cached one until the first survey answers.
    shown: Option<CachedObservation>,
    /// `shown` came from the cache and no survey has answered yet this session.
    from_cache: bool,
    /// Ordered for display.
    decisions: Vec<Decision>,
    /// The last survey's failure, when it produced no Observation.
    failure: Option<SurveyFailure>,
    selected: Option<WorkItemId>,
    now: Option<Timestamp>,
}

impl App {
    pub fn new(config: Result<Config, ConfigError>) -> Self {
        Self {
            config,
            running: None,
            next_survey: 0,
            shown: None,
            from_cache: false,
            decisions: Vec::new(),
            failure: None,
            selected: None,
            now: None,
        }
    }

    pub fn handle(&mut self, event: Event) -> (ViewModel, Vec<Effect>) {
        let effects = match event {
            Event::Started { now } => {
                self.now = Some(now);
                self.start()
            }
            Event::Tick { now } => {
                self.now = Some(now);
                Vec::new()
            }
            Event::Key(key) => self.key(key),
            Event::SurveyFinished { id, outcome, now } => {
                self.now = Some(now);
                self.survey_finished(id, outcome, now)
            }
            Event::CacheLoaded { outcome } => self.cache_loaded(outcome),
            Event::CacheStored { outcome } => cache_stored(outcome),
        };
        (self.view(), effects)
    }

    /// The current ViewModel, without changing anything.
    pub fn view(&self) -> ViewModel {
        view::build(&view::State {
            config: &self.config,
            surveying: self.running.is_some(),
            observation: self.shown.as_ref().map(|s| &s.observation),
            carried: self.shown.as_ref().map_or(&[], |s| s.carried.as_slice()),
            from_cache: self.from_cache,
            decisions: &self.decisions,
            failure: self.failure.as_ref(),
            selected: self.selected_index(),
            now: self.now,
        })
    }

    fn start(&mut self) -> Vec<Effect> {
        match &self.config {
            Ok(config) => {
                let request = SurveyRequest {
                    owners: config.owners.clone(),
                };
                // The cache is read first, so it shows before the survey answers.
                let mut effects = vec![Effect::LoadCache];
                effects.extend(self.begin_survey(request));
                effects
            }
            Err(error) => vec![Effect::log(LogLevel::Error, format!("config: {error}"))],
        }
    }

    fn begin_survey(&mut self, request: SurveyRequest) -> Vec<Effect> {
        let id = SurveyId(self.next_survey);
        self.next_survey += 1;
        self.running = Some(id);
        vec![
            Effect::log(
                LogLevel::Info,
                format!(
                    "survey {} started for {} Owner(s): {}",
                    id.0,
                    request.owners.len(),
                    request.owners.join(", ")
                ),
            ),
            Effect::Survey { id, request },
        ]
    }

    fn key(&mut self, key: Key) -> Vec<Effect> {
        match key {
            Key::Char('q') | Key::Interrupt => vec![Effect::Quit],
            Key::Down => {
                self.move_selection(1);
                Vec::new()
            }
            Key::Up => {
                self.move_selection(-1);
                Vec::new()
            }
            Key::Char('r') => self.refresh(),
            Key::Enter | Key::Esc | Key::Char(_) => Vec::new(),
        }
    }

    fn survey_finished(
        &mut self,
        id: SurveyId,
        outcome: Result<Box<Observation>, SurveyFailure>,
        now: Timestamp,
    ) -> Vec<Effect> {
        if self.running != Some(id) {
            return vec![Effect::log(
                LogLevel::Warn,
                format!(
                    "ignored the answer to survey {}, which is no longer current",
                    id.0
                ),
            )];
        }
        self.running = None;
        match outcome {
            Ok(observation) => {
                let log = survey_log(id, &observation);
                let merged = cache::merge(self.shown.take(), *observation);
                let carried = carried_log(&merged);
                self.show(merged.clone(), now);
                self.from_cache = false;
                self.failure = None;
                let mut effects = vec![log];
                effects.extend(carried);
                effects.push(Effect::StoreCache {
                    cached: Box::new(merged),
                });
                effects
            }
            Err(failure) => {
                let log = Effect::log(
                    LogLevel::Error,
                    format!("survey {} failed: {failure}", id.0),
                );
                // The last good Observation, if any, stays on screen.
                self.failure = Some(failure);
                vec![log]
            }
        }
    }

    /// `r`: survey again, unless a survey is already running or there is no config.
    fn refresh(&mut self) -> Vec<Effect> {
        if self.running.is_some() {
            return vec![Effect::log(
                LogLevel::Info,
                "refresh asked for while a survey is running; waiting for that one",
            )];
        }
        match &self.config {
            Ok(config) => {
                let request = SurveyRequest {
                    owners: config.owners.clone(),
                };
                self.begin_survey(request)
            }
            Err(error) => vec![Effect::log(
                LogLevel::Warn,
                format!("refresh asked for, but the config is unusable: {error}"),
            )],
        }
    }

    fn cache_loaded(
        &mut self,
        outcome: Result<Option<Box<CachedObservation>>, CacheError>,
    ) -> Vec<Effect> {
        let cached = match outcome {
            Ok(Some(cached)) => cached,
            Ok(None) => {
                return vec![Effect::log(
                    LogLevel::Info,
                    "no cached Observation yet; waiting for the first survey",
                )];
            }
            Err(error) => {
                return vec![Effect::log(
                    LogLevel::Warn,
                    format!("ignored the cache: {error}; surveying afresh"),
                )];
            }
        };
        if self.shown.is_some() {
            return vec![Effect::log(
                LogLevel::Info,
                "ignored the cache: a survey already answered",
            )];
        }
        let Ok(config) = &self.config else {
            return Vec::new();
        };
        let cached = cached.restricted_to(&config.owners);
        let log = Effect::log(
            LogLevel::Info,
            format!(
                "showing the cached Observation of {} from {}",
                cached.observation.viewer.login, cached.observation.observed_at
            ),
        );
        let now = self.now.unwrap_or(cached.observation.observed_at);
        self.show(cached, now);
        self.from_cache = true;
        vec![log]
    }

    /// Shows `shown`, deciding its Work Items as of `now`, keeping the selection.
    fn show(&mut self, shown: CachedObservation, now: Timestamp) {
        let observation = &shown.observation;
        self.decisions = ordering::order(decide::decide(observation, &observation.viewer, now));
        self.shown = Some(shown);
        self.keep_selection();
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.decisions.iter().position(|d| &d.item.id == selected)
    }

    /// Keeps the selection on the same Work Item if it still exists, else the first.
    fn keep_selection(&mut self) {
        if self.selected_index().is_none() {
            self.selected = self.decisions.first().map(|d| d.item.id.clone());
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let Some(last) = self.decisions.len().checked_sub(1) else {
            return;
        };
        let target = match self.selected_index() {
            Some(i) => i.saturating_add_signed(delta).min(last),
            None => 0,
        };
        self.selected = self.decisions.get(target).map(|d| d.item.id.clone());
    }
}

fn cache_stored(outcome: Result<(), CacheError>) -> Vec<Effect> {
    match outcome {
        Ok(()) => Vec::new(),
        Err(error) => vec![Effect::log(
            LogLevel::Warn,
            format!("could not save the cache: {error}"),
        )],
    }
}

/// Logs which sources show data kept from an earlier survey.
fn carried_log(merged: &CachedObservation) -> Option<Effect> {
    if merged.carried.is_empty() {
        return None;
    }
    let kept: Vec<String> = merged
        .carried
        .iter()
        .map(|c| format!("{} from {}", c.source.label(), c.observed_at))
        .collect();
    Some(Effect::log(
        LogLevel::Warn,
        format!("kept earlier data for: {}", kept.join("; ")),
    ))
}

fn survey_log(id: SurveyId, observation: &Observation) -> Effect {
    let incomplete: Vec<String> = observation
        .coverage
        .incomplete()
        .map(|e| format!("{} {}", e.source.label(), e.state.describe()))
        .collect();
    let troubled = observation
        .coverage
        .incomplete()
        .any(|e| e.state != SourceState::NotYetFetched);
    let level = if troubled {
        LogLevel::Warn
    } else {
        LogLevel::Info
    };
    Effect::log(
        level,
        format!(
            "survey {} finished: {} PR(s) in {} Repo(s); incomplete: [{}]",
            id.0,
            observation.pull_requests.len(),
            observation.repos.len(),
            incomplete.join("; ")
        ),
    )
}
