//! Seam 2: drive the App with events; Effects are performed through fake ports
//! (with the same `perform` the real worker uses) and their Events fed back.

use super::{
    build::now,
    fakes::{
        FakeCache, FakeGitHub, FakeJobs, FakeLauncher, FakeLinear, FakeLinearWriter, FakeUpstreams,
        FakeWorkspace, FixedClock,
    },
};
use den::{
    app::{App, Effect, Event, Key, ViewModel},
    config::{Config, ConfigError, LinearWorkflow, Location},
    domain::Observation,
    ports::{Ports, SurveyFailure},
    runtime,
};
use std::{collections::BTreeMap, sync::Arc};

pub struct Harness {
    pub app: App,
    pub github: Arc<FakeGitHub>,
    pub linear: Arc<FakeLinear>,
    /// The two Linear writes (#38): recorded, never sent anywhere.
    pub linear_writer: Arc<FakeLinearWriter>,
    pub cache: Arc<FakeCache>,
    pub workspace: Arc<FakeWorkspace>,
    pub launcher: Arc<FakeLauncher>,
    pub jobs: Arc<FakeJobs>,
    pub upstreams: Arc<FakeUpstreams>,
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
        linear: Vec::new(),
        skips: den::domain::Skips::default(),
    }
}

/// A Linear Workflow for `owner` on `workspace`, its key in
/// `DEN_TEST_LINEAR_KEY`, every setting at its config default except the
/// review switches, which are all on: the strictest rules, as spec 0002 had
/// them. The config's own defaults follow GitHub since ADR-0009 (#73) and are
/// tested in `tests/config.rs` and `tests/decide_review_rules.rs`.
pub fn linear_workflow(owner: &str, workspace: &str) -> LinearWorkflow {
    LinearWorkflow {
        owner: owner.to_owned(),
        workspace: workspace.to_owned(),
        api_key_env: "DEN_TEST_LINEAR_KEY".to_owned(),
        api_key_env_at: Location { line: 1, column: 1 },
        start_teams: Vec::new(),
        include_backlog: false,
        active_cycle_only: false,
        new_work_repos: Vec::new(),
        held_repos: Vec::new(),
        landing_order: Vec::new(),
        reviewers: Vec::new(),
        slack_channel: None,
        in_progress_state: None,
        release_state: None,
        parked_states: Vec::new(),
        exact_head_approval: true,
        comment_reviews_need_rereview: true,
        pending_requests_hold_merge: true,
    }
}

/// `owners`, with a Linear Workflow for each of `linear` (Owner, workspace).
pub fn config_with_linear(owners: &[&str], linear: &[(&str, &str)]) -> Config {
    Config {
        linear: linear
            .iter()
            .map(|(owner, workspace)| linear_workflow(owner, workspace))
            .collect(),
        ..config(owners)
    }
}

impl Harness {
    pub fn new(config: Result<Config, ConfigError>) -> Self {
        Self::with_cache(config, FakeCache::default())
    }

    /// A harness whose cache starts as `cache`.
    pub fn with_cache(config: Result<Config, ConfigError>, cache: FakeCache) -> Self {
        Self::with_stores(config, cache, Arc::new(FakeUpstreams::default()))
    }

    /// A harness whose cache starts as `cache` and whose App reads its Upstream
    /// choices from `upstreams` at startup, as `main` does (a restart is a new
    /// harness over the same `upstreams`).
    pub fn with_stores(
        config: Result<Config, ConfigError>,
        cache: FakeCache,
        upstreams: Arc<FakeUpstreams>,
    ) -> Self {
        let github = Arc::new(FakeGitHub::default());
        let linear = Arc::new(FakeLinear::default());
        let linear_writer = Arc::new(FakeLinearWriter::default());
        let cache = Arc::new(cache);
        let workspace = Arc::new(FakeWorkspace::default());
        let launcher = Arc::new(FakeLauncher::default());
        let jobs = Arc::new(FakeJobs::default());
        let app =
            App::new(config).with_upstreams(den::ports::UpstreamStore::load(upstreams.as_ref()));
        let view = app.view();
        Self {
            ports: Ports {
                github: github.clone(),
                linear: linear.clone(),
                linear_writer: linear_writer.clone(),
                cache: cache.clone(),
                workspace: workspace.clone(),
                launcher: launcher.clone(),
                jobs: jobs.clone(),
                upstreams: upstreams.clone(),
                clock: Arc::new(FixedClock(now())),
            },
            app,
            github,
            linear,
            linear_writer,
            cache,
            workspace,
            launcher,
            jobs,
            upstreams,
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

    /// From now on the ports' clock reads `now` (time passed between rounds).
    pub fn set_clock(&mut self, now: jiff::Timestamp) {
        self.ports.clock = Arc::new(FixedClock(now));
    }

    /// den starts, surveys, and the answer arrives.
    pub fn start(&mut self) -> &ViewModel {
        self.settle(Event::Started { now: now() })
    }

    /// Performs one Effect through the fake ports and handles the Events it
    /// produces. Their Effects are returned (and recorded), not performed.
    pub fn perform(&mut self, effect: &Effect) -> Vec<Effect> {
        let mut follow_ups = Vec::new();
        for event in den::ports::perform(effect, &self.ports) {
            follow_ups.extend(self.send(event));
        }
        follow_ups
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
