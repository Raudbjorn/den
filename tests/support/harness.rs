//! Seam 2: drive the App with events; Effects are performed through fake ports
//! (with the same `perform` the real worker uses) and their Events fed back.

use super::{
    build::now,
    fakes::{self, FakeGitHub},
};
use den::{
    app::{App, Effect, Event, Key, ViewModel},
    config::{Config, ConfigError},
    domain::Observation,
    ports::{Ports, SurveyFailure},
    runtime,
};
use std::{collections::BTreeMap, sync::Arc};

pub struct Harness {
    pub app: App,
    pub github: Arc<FakeGitHub>,
    ports: Ports,
    /// Every Effect the App emitted, in order.
    pub effects: Vec<Effect>,
    pub view: ViewModel,
}

pub fn config(owners: &[&str]) -> Config {
    Config {
        owners: owners.iter().map(|o| (*o).to_owned()).collect(),
        main_clone_root: None,
        main_clones: BTreeMap::new(),
    }
}

impl Harness {
    pub fn new(config: Result<Config, ConfigError>) -> Self {
        let github = Arc::new(FakeGitHub::default());
        let app = App::new(config);
        let view = app.view();
        Self {
            ports: fakes::ports(github.clone(), now()),
            app,
            github,
            effects: Vec::new(),
            view,
        }
    }

    pub fn with_owners(owners: &[&str]) -> Self {
        Self::new(Ok(config(owners)))
    }

    /// Queues the next survey's answer.
    pub fn answer(&self, outcome: Result<Observation, SurveyFailure>) -> &Self {
        self.github.push_answer(outcome);
        self
    }

    /// Sends `event` and performs everything it leads to, until idle.
    pub fn settle(&mut self, event: Event) -> &ViewModel {
        let (view, effects) = runtime::settle(&mut self.app, event, &self.ports);
        self.view = view;
        self.effects.extend(effects);
        &self.view
    }

    /// Sends `event` without performing its Effects; returns them.
    pub fn send(&mut self, event: Event) -> Vec<Effect> {
        let (view, effects) = self.app.handle(event);
        self.view = view;
        self.effects.extend(effects.iter().cloned());
        effects
    }

    /// den starts, surveys, and the answer arrives.
    pub fn start(&mut self) -> &ViewModel {
        self.settle(Event::Started { now: now() })
    }

    pub fn key(&mut self, key: Key) -> Vec<Effect> {
        self.send(Event::Key(key))
    }

    /// The rows' `(repo, reference, step label)`, in list order.
    pub fn rows(&self) -> Vec<(String, String, String)> {
        self.view
            .list
            .rows
            .iter()
            .map(|r| (r.repo.clone(), r.reference.clone(), r.step_label.clone()))
            .collect()
    }

    pub fn survey_effects(&self) -> usize {
        self.effects
            .iter()
            .filter(|e| matches!(e, Effect::Survey { .. }))
            .count()
    }
}
