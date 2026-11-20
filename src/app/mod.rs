//! The App (test seam 2): `handle(Event) -> (ViewModel, Effects)`.
//!
//! The App holds den's state and decides what to show and what to ask the ports
//! for. It does no I/O and reads no clock: time arrives in events.

mod effect;
mod event;
mod scope;
mod view;
pub mod view_model;

pub use effect::{Effect, LogLevel, LogRecord};
pub use event::{Event, Key, SurveyId};
pub use scope::Focus;
pub use view_model::ViewModel;

use crate::{
    config::{Config, ConfigError},
    decide::{self, Decision},
    domain::{Observation, SourceState, WorkItemId},
    ordering,
    ports::{SurveyFailure, SurveyRequest},
};
use jiff::Timestamp;
use scope::ScopeState;

pub struct App {
    config: Result<Config, ConfigError>,
    /// The survey in flight, if any.
    running: Option<SurveyId>,
    next_survey: u64,
    observation: Option<Observation>,
    /// Every Decision, ordered for display.
    ordered: Vec<Decision>,
    /// The ordered Decisions in the current Scope: what the list shows.
    decisions: Vec<Decision>,
    /// The Scope, and the sidebar's focus and cursor.
    scope: ScopeState,
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
            observation: None,
            ordered: Vec::new(),
            decisions: Vec::new(),
            scope: ScopeState::default(),
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
        };
        (self.view(), effects)
    }

    /// The current ViewModel, without changing anything.
    pub fn view(&self) -> ViewModel {
        view::build(&view::State {
            config: &self.config,
            surveying: self.running.is_some(),
            observation: self.observation.as_ref(),
            decisions: &self.decisions,
            all_decisions: &self.ordered,
            owners: self.owners(),
            scope: &self.scope,
            sidebar: self.sidebar_entries(),
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
                self.begin_survey(request)
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
        let entries = self.sidebar_entries();
        let owners = self.owners().to_vec();
        match self.scope.key(key, &owners, &entries) {
            scope::Outcome::Unhandled => {}
            scope::Outcome::Moved => return Vec::new(),
            scope::Outcome::Changed => {
                self.apply_scope();
                return Vec::new();
            }
        }
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
            Key::Enter | Key::Esc | Key::Char(_) | Key::Left | Key::Right | Key::Tab => Vec::new(),
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
                self.ordered =
                    ordering::order(decide::decide(&observation, &observation.viewer, now));
                self.observation = Some(*observation);
                self.failure = None;
                self.apply_scope();
                vec![log]
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

    /// The watched Owners: the config's, else the last Observation's.
    fn owners(&self) -> &[String] {
        match (&self.config, &self.observation) {
            (Ok(config), _) => &config.owners,
            (Err(_), Some(observation)) => &observation.owners,
            (Err(_), None) => &[],
        }
    }

    fn sidebar_entries(&self) -> Vec<ordering::Scope> {
        scope::entries(self.owners(), self.observation.as_ref(), &self.ordered)
    }

    /// Narrows the list to the Scope, keeping the selection when it is still in it.
    fn apply_scope(&mut self) {
        self.decisions = ordering::in_scope(&self.ordered, &self.scope.scope)
            .cloned()
            .collect();
        self.keep_selection();
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.decisions
            .iter()
            .position(|d| ordering::same_work_item(&d.item.id, selected))
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
