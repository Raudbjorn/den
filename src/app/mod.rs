//! The App (test seam 2): `handle(Event) -> (ViewModel, Effects)`.
//!
//! The App holds den's state and decides what to show and what to ask the ports
//! for. It does no I/O and reads no clock: time arrives in events.

mod editor;
mod effect;
mod event;
mod launch;
mod overlay;
mod scope;
mod upstream;
mod view;
pub mod view_model;

pub use effect::{Effect, LogLevel, LogRecord};
pub use event::{Event, Key, LaunchId, SurveyId};
pub use scope::Focus;
pub use view_model::ViewModel;

use crate::{
    cache::{self, CacheError, CachedObservation},
    config::{Config, ConfigError, LinearWorkflow},
    decide::{self, Decision},
    domain::{
        Coverage, LinearSourceKind, Observation, Policies, Source, SourceKind, SourceState,
        UpstreamTargets, Viewer, Viewers, WorkItemId, ids::same_login,
    },
    handoff::{
        JobCheck, JobsCheck,
        prompts::{Prompts, PromptsError},
    },
    layout::{Layout, TerminalSize},
    ordering,
    ports::{LinearSurvey, LinearSurveyRequest, SurveyFailure, SurveyRequest},
};
use jiff::Timestamp;
use scope::ScopeState;
use std::path::PathBuf;

/// The sheet over the screen, if any, with its state.
#[derive(Debug, Clone)]
enum OverlayState {
    /// The detail as a sheet on narrow terminals (#10).
    DetailSheet,
    /// The launch sheet (#11).
    Launch(Box<launch::Sheet>),
}

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
    /// Every Decision, ordered for display.
    ordered: Vec<Decision>,
    /// The ordered Decisions in the current Scope: what the list shows.
    decisions: Vec<Decision>,
    /// The Scope, and the sidebar's focus and cursor.
    scope: ScopeState,
    /// The last survey's failure, when it produced no Observation.
    failure: Option<SurveyFailure>,
    selected: Option<WorkItemId>,
    detail_page: i64,
    now: Option<Timestamp>,
    /// The terminal's size, as last reported (`Event::Resized`).
    size: TerminalSize,
    /// The sheet drawn over the screen, if any.
    overlay: Option<OverlayState>,
    /// The prompts file's templates, or why it is unusable.
    prompts: Result<Prompts, PromptsError>,
    /// Where to search for Main Clones when the config sets no `main_clone_root`.
    default_clone_root: Option<PathBuf>,
    next_launch: u64,
    /// The recorded Jobs' worktree states (#14), newest first; refreshed at
    /// start, on `r`, after a launch, and per Work Item by a sheet's inspection.
    job_checks: Vec<JobCheck>,
    /// The forks whose target is their Upstream (#13).
    upstreams: UpstreamTargets,
    /// Why the stored Upstream choices couldn't be read at startup, if they
    /// couldn't; `u` then changes nothing.
    upstreams_unreadable: Option<String>,
    /// The last thing that went wrong with a `u` (not saved, Upstream not visible).
    upstream_problem: Option<String>,
    /// Each Owner's Linear Workflow policy for the decision engine, from the
    /// config: the empty policy for an Owner without a Linear Workflow.
    policies: Policies,
    /// The Linear survey in flight, if any (#32).
    linear_running: Option<SurveyId>,
    /// The latest Linear survey's answer this session: spliced into every
    /// Observation den shows (#34 will merge it per source with the cache).
    linear: Option<LinearSurvey>,
    /// A Linear answer was shown but not yet saved: the GitHub answer of the
    /// same survey was still to come, and saves both (or, if it fails, the
    /// failure does).
    linear_unsaved: bool,
}

impl App {
    pub fn new(config: Result<Config, ConfigError>) -> Self {
        let policies = config.as_ref().map(Config::policies).unwrap_or_default();
        Self {
            config,
            running: None,
            next_survey: 0,
            shown: None,
            from_cache: false,
            ordered: Vec::new(),
            decisions: Vec::new(),
            scope: ScopeState::default(),
            failure: None,
            selected: None,
            detail_page: 0,
            now: None,
            size: TerminalSize::DESIGN,
            overlay: None,
            prompts: Ok(Prompts::default()),
            default_clone_root: None,
            next_launch: 0,
            job_checks: Vec::new(),
            upstreams: UpstreamTargets::default(),
            policies,
            linear_running: None,
            linear: None,
            linear_unsaved: false,
            upstreams_unreadable: None,
            upstream_problem: None,
        }
    }

    /// Sets the launch settings that don't come from the config: the prompts
    /// file (or why it is unusable) and the default Main Clone search root.
    pub fn with_launch_settings(
        mut self,
        prompts: Result<Prompts, PromptsError>,
        default_clone_root: Option<PathBuf>,
    ) -> Self {
        self.prompts = prompts;
        self.default_clone_root = default_clone_root;
        self
    }

    pub fn handle(&mut self, event: Event) -> (ViewModel, Vec<Effect>) {
        let effects = match event {
            Event::Started { now } => {
                self.now = Some(now);
                self.start()
            }
            Event::Tick { now } => {
                self.now = Some(now);
                self.decide_shown(now);
                Vec::new()
            }
            Event::Key(key) => self.key(key),
            Event::EffectRejected { effect, reason } => self.effect_rejected(*effect, reason),
            Event::Resized { cols, rows } => self.resized(TerminalSize { cols, rows }),
            Event::SurveyFinished { id, outcome, now } => {
                self.now = Some(now);
                self.survey_finished(id, outcome, now)
            }
            Event::LinearSurveyFinished { id, survey, now } => {
                self.now = Some(now);
                self.linear_survey_finished(id, *survey, now)
            }
            Event::CacheLoaded { outcome } => self.cache_loaded(outcome),
            Event::CacheStored { outcome } => cache_stored(outcome),
            Event::LaunchInspected { id, view } => self.launch_inspected(id, view),
            Event::LaunchFinished { id, report } => self.launch_finished(id, report),
            Event::JobsChecked { outcome } => self.jobs_checked(outcome),
            Event::UpstreamsStored { outcome } => self.upstreams_stored(outcome),
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
            all_decisions: &self.ordered,
            owners: self.owners(),
            scope: &self.scope,
            sidebar: self.sidebar_entries(),
            failure: self.failure.as_ref(),
            selected: self.selected_index(),
            detail_page: self.detail_page,
            now: self.now,
            layout: Layout::for_size(self.size),
            overlay: self.overlay.as_ref(),
            job_checks: &self.job_checks,
            upstreams: &self.upstreams,
            upstream_note: self.upstream_note(),
        })
    }

    fn start(&mut self) -> Vec<Effect> {
        match &self.config {
            Ok(config) => {
                let request = SurveyRequest {
                    owners: config.owners.clone(),
                    upstreams: self.upstreams.clone(),
                };
                // The cache is read first, so it shows before the survey answers;
                // then the Jobs (local and bounded), so their markers don't wait
                // for gh on the single worker.
                let mut effects = vec![Effect::LoadCache, Effect::CheckJobs];
                if let Some(error) = &self.upstreams_unreadable {
                    effects.push(Effect::log(
                        LogLevel::Warn,
                        format!("ignored the Upstream choices: {error}; every fork targets itself"),
                    ));
                }
                effects.extend(self.begin_survey(request));
                effects
            }
            Err(error) => vec![Effect::log(LogLevel::Error, format!("config: {error}"))],
        }
    }

    fn effect_rejected(&mut self, effect: Effect, reason: String) -> Vec<Effect> {
        let operation = match effect {
            Effect::Survey { id, .. } => {
                if self.running == Some(id) {
                    self.running = None;
                    self.failure = Some(SurveyFailure::Interrupted {
                        detail: reason.clone(),
                    });
                }
                "survey"
            }
            Effect::InspectLaunch { id, .. }
            | Effect::Launch { id, .. }
            | Effect::Reopen { id, .. } => {
                self.launch_not_queued(id, reason.clone());
                "launch request"
            }
            Effect::StoreUpstreams { .. } => {
                self.upstream_problem = Some(format!(
                    "Upstream choice not saved, so it lasts until den quits: {reason}"
                ));
                "save Upstream choices"
            }
            Effect::SurveyLinear { id, .. } => {
                let mut effects = vec![Effect::log(
                    LogLevel::Error,
                    format!("could not queue Linear survey: {reason}"),
                )];
                if self.linear_running == Some(id) {
                    self.linear_running = None;
                    effects.extend(self.linear_not_asked(id, &reason));
                }
                return effects;
            }
            Effect::LoadCache => "read cache",
            Effect::StoreCache { .. } => "save cache",
            Effect::CheckJobs => "check Jobs",
            Effect::Quit => "quit",
            Effect::Log(_) => "log",
        };
        vec![Effect::log(
            LogLevel::Error,
            format!("could not queue {operation}: {reason}"),
        )]
    }

    /// The Linear survey `id` couldn't be queued: Linear wasn't asked, so
    /// every Linear source reads as failed for this survey (never as the
    /// previous answer, complete and current).
    fn linear_not_asked(&mut self, id: SurveyId, reason: &str) -> Vec<Effect> {
        let mut survey = LinearSurvey::default();
        for source in self.linear_sources() {
            survey.coverage.record(
                source,
                SourceState::Failed {
                    error: format!("den could not ask Linear: {reason}"),
                },
            );
        }
        match self.now {
            Some(now) => self.linear_answered(id, survey, now),
            None => {
                self.linear = Some(survey);
                Vec::new()
            }
        }
    }

    /// Starts a survey: Linear when any Owner has a Linear Workflow, and
    /// GitHub. The worker runs the Linear survey on its own lane, so a slow
    /// Linear never holds GitHub's answer back; either may answer first.
    /// (Run in order, as `settle` does, Linear's answer is in hand when
    /// GitHub's arrives.) Without Linear Workflows den asks Linear nothing.
    fn begin_survey(&mut self, request: SurveyRequest) -> Vec<Effect> {
        let id = SurveyId(self.next_survey);
        self.next_survey += 1;
        self.running = Some(id);
        let mut effects = vec![Effect::log(
            LogLevel::Info,
            format!(
                "survey {} started for {} Owner(s): {}",
                id.0,
                request.owners.len(),
                request.owners.join(", ")
            ),
        )];
        let workflows = self
            .config
            .as_ref()
            .map(|c| c.linear.clone())
            .unwrap_or_default();
        if !workflows.is_empty() {
            self.linear_running = Some(id);
            effects.push(Effect::log(
                LogLevel::Info,
                format!(
                    "Linear survey {} started for the Linear Workflow(s) of {}",
                    id.0,
                    workflows
                        .iter()
                        .map(|w| w.owner.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
            effects.push(Effect::SurveyLinear {
                id,
                request: LinearSurveyRequest { workflows },
            });
        }
        effects.push(Effect::Survey { id, request });
        effects
    }

    fn key(&mut self, key: Key) -> Vec<Effect> {
        // The launch sheet claims every key.
        if let Some(OverlayState::Launch(_)) = &self.overlay
            && let Some(OverlayState::Launch(sheet)) = self.overlay.take()
        {
            return self.launch_key(sheet, key);
        }
        if matches!(key, Key::PageUp | Key::PageDown) {
            self.detail_page =
                self.detail_page
                    .saturating_add(if key == Key::PageDown { 1 } else { -1 });
            return Vec::new();
        }
        // An open sheet claims its keys before anything else.
        if self.overlay.is_some()
            && let Some(effects) = self.overlay_key(key)
        {
            return effects;
        }
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
        // `Enter` in the list opens the detail sheet when there is no pane.
        if let Some(effects) = self.overlay_key(key) {
            return effects;
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
            Key::Char('r') => self.refresh(),
            Key::Char('c') => self.open_launch_sheet(),
            Key::Char('u') if self.scope.focus == Focus::Sidebar => self.toggle_upstream(),
            Key::Enter
            | Key::Esc
            | Key::Char(_)
            | Key::Left
            | Key::Right
            | Key::Tab
            | Key::Backspace
            | Key::Delete
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown => Vec::new(),
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
                let mut merged = cache::merge(self.shown.take(), *observation);
                self.with_linear(&mut merged);
                let carried = carried_log(&merged);
                self.show(merged.clone(), now);
                self.from_cache = false;
                self.failure = None;
                self.linear_unsaved = false;
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
                let mut effects = vec![log];
                match &self.shown {
                    // A Linear answer shown since the last save is saved now.
                    Some(shown) if self.linear_unsaved && has_github_viewer(shown) => {
                        effects.push(Effect::StoreCache {
                            cached: Box::new(shown.clone()),
                        });
                    }
                    Some(_) => {}
                    // Nothing shown yet: Linear's answer, if any, still shows.
                    None => self.show_linear_only(&failure, now),
                }
                self.linear_unsaved = false;
                self.failure = Some(failure);
                effects
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
                    upstreams: self.upstreams.clone(),
                };
                let mut effects = vec![Effect::CheckJobs];
                effects.extend(self.begin_survey(request));
                effects
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
        let mut cached = cached.restricted_to(&config.owners);
        fit_linear(&mut cached, &config.linear);
        if self.linear.is_some() {
            // Linear answered before the cache was read: its answer is newer.
            self.with_linear(&mut cached);
        }
        self.record_unanswered_linear(&mut cached.observation.coverage);
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
        self.shown = Some(shown);
        self.decide_shown(now);
    }

    /// Decides what is shown again (the Upstream choices changed).
    fn redecide(&mut self) {
        if let Some(now) = self
            .now
            .or_else(|| self.shown.as_ref().map(|s| s.observation.observed_at))
        {
            self.decide_shown(now);
        }
    }

    fn decide_shown(&mut self, now: Timestamp) {
        let Some(shown) = &self.shown else {
            return;
        };
        let observation = &shown.observation;
        self.ordered = ordering::order(decide::decide_with(
            observation,
            &Viewers::of(observation),
            &self.policies,
            &self.upstreams,
            now,
        ));
        self.apply_scope();
    }

    /// The watched Owners: the config's, else the last Observation's.
    fn owners(&self) -> &[String] {
        match (&self.config, self.shown.as_ref().map(|s| &s.observation)) {
            (Ok(config), _) => &config.owners,
            (Err(_), Some(observation)) => &observation.owners,
            (Err(_), None) => &[],
        }
    }

    fn sidebar_entries(&self) -> Vec<ordering::Scope> {
        scope::entries(
            self.owners(),
            self.shown.as_ref().map(|s| &s.observation),
            &self.ordered,
        )
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
            self.detail_page = 0;
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
        let selected = self.decisions.get(target).map(|d| d.item.id.clone());
        if selected != self.selected {
            self.detail_page = 0;
        }
        self.selected = selected;
    }
}

impl App {
    /// The Jobs' worktrees were checked: the list and detail mark them. On an
    /// error the earlier checks stay (and `c` refuses until the records read).
    fn jobs_checked(&mut self, outcome: Result<Box<JobsCheck>, String>) -> Vec<Effect> {
        match outcome {
            Ok(check) => {
                let live = check.checks.iter().filter(|c| c.is_live()).count();
                let unknown = check
                    .checks
                    .iter()
                    .filter(|c| matches!(c.state, crate::handoff::WorktreeState::Unknown(_)))
                    .count();
                let log = Effect::log(
                    LogLevel::Info,
                    format!(
                        "checked {} Job(s): {live} live, {unknown} unknown; {} older not checked",
                        check.checks.len(),
                        check.not_checked
                    ),
                );
                self.job_checks = check.checks;
                vec![log]
            }
            Err(error) => vec![Effect::log(
                LogLevel::Warn,
                format!("could not check the Jobs: {error}"),
            )],
        }
    }
}

impl App {
    /// A Linear survey answered: it replaces the Linear part of what is shown
    /// (and is spliced into every later GitHub answer until the next one).
    fn linear_survey_finished(
        &mut self,
        id: SurveyId,
        survey: LinearSurvey,
        now: Timestamp,
    ) -> Vec<Effect> {
        if self.linear_running != Some(id) {
            return vec![Effect::log(
                LogLevel::Warn,
                format!(
                    "ignored the answer to Linear survey {}, which is no longer current",
                    id.0
                ),
            )];
        }
        self.linear_running = None;
        self.linear_answered(id, survey, now)
    }

    /// Takes the Linear answer of survey `id` into what is shown. It is saved
    /// at once unless the same survey's GitHub answer is still to come (that
    /// one saves both, so a refresh writes the cache once).
    fn linear_answered(
        &mut self,
        id: SurveyId,
        survey: LinearSurvey,
        now: Timestamp,
    ) -> Vec<Effect> {
        let mut effects = vec![linear_log(id, &survey)];
        self.linear = Some(survey);
        let Some(mut shown) = self.shown.take() else {
            // GitHub already failed with nothing shown: show Linear's answer.
            if self.running.is_none()
                && let Some(failure) = self.failure.clone()
            {
                self.show_linear_only(&failure, now);
            }
            return effects;
        };
        self.with_linear(&mut shown);
        self.show(shown.clone(), now);
        if !has_github_viewer(&shown) {
            // The Linear-only stand-in: never saved.
        } else if self.running == Some(id) {
            self.linear_unsaved = true;
        } else {
            effects.push(Effect::StoreCache {
                cached: Box::new(shown),
            });
        }
        effects
    }

    /// With no GitHub answer to show, shows the Linear answer alone, if it
    /// holds Linear Issues. GitHub's viewer is unknown and its failure is
    /// coverage, so nothing reads as complete. It is never saved (the cache
    /// is per GitHub viewer); the next GitHub answer starts afresh.
    fn show_linear_only(&mut self, failure: &SurveyFailure, now: Timestamp) {
        let Ok(config) = &self.config else {
            return;
        };
        if self
            .linear
            .as_ref()
            .is_none_or(|l| l.part.issues.is_empty())
        {
            return;
        }
        let unknown = Viewer {
            login: String::new(),
            teams: Vec::new(),
        };
        let mut observation = Observation::empty(now, unknown, config.owners.clone());
        observation.coverage.record(
            Source::Viewer,
            SourceState::Failed {
                error: failure.to_string(),
            },
        );
        let mut shown = CachedObservation::fresh(observation);
        self.with_linear(&mut shown);
        self.show(shown, now);
    }

    /// Gives `shown` this session's Linear part: the latest Linear survey's,
    /// or, before one answers, Linear sources not yet fetched for each Owner
    /// with a Linear Workflow (never an empty part that reads as complete).
    /// Without Linear Workflows, no Linear part at all.
    fn with_linear(&self, shown: &mut CachedObservation) {
        without_linear(shown);
        if self.config.is_err() {
            return;
        }
        let observation = &mut shown.observation;
        if let Some(survey) = &self.linear {
            observation.linear = survey.part.clone();
            for entry in survey.coverage.entries() {
                observation
                    .coverage
                    .record(entry.source.clone(), entry.state.clone());
            }
        }
        // Whatever the survey said (or before one answered), every Linear
        // Workflow's sources are recorded: one den has no word on is not
        // yet fetched, never silently complete.
        self.record_unanswered_linear(&mut observation.coverage);
    }

    /// Records every Linear source of each Linear Workflow that `coverage`
    /// has no word on as not yet fetched, so that one den hasn't heard from
    /// never reads as complete. Without Linear Workflows, records nothing.
    fn record_unanswered_linear(&self, coverage: &mut Coverage) {
        for source in self.linear_sources() {
            if !coverage.entries().iter().any(|e| e.source.same(&source)) {
                coverage.record(source, SourceState::NotYetFetched);
            }
        }
    }

    /// Every Linear source of every configured Linear Workflow.
    fn linear_sources(&self) -> Vec<Source> {
        let Ok(config) = &self.config else {
            return Vec::new();
        };
        config
            .linear
            .iter()
            .flat_map(|workflow| {
                [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues].map(|kind| {
                    Source::Owner {
                        owner: workflow.owner.clone(),
                        kind: SourceKind::Linear(kind),
                    }
                })
            })
            .collect()
    }
}

/// Whether `shown` holds a GitHub answer (or a cached one): only then does
/// it have a GitHub viewer to be saved under. GitHub logins are never
/// empty; the Linear-only stand-in's is.
fn has_github_viewer(shown: &CachedObservation) -> bool {
    !shown.observation.viewer.login.is_empty()
}

/// Fits a cached Linear part to the configured Linear Workflows: drops the
/// Linear facts, Linear coverage and Linear ages of every Owner that has no
/// Linear Workflow now, or whose workflow names another workspace than the
/// cached data was read in.
fn fit_linear(shown: &mut CachedObservation, workflows: &[LinearWorkflow]) {
    let workspace_of = |owner: &str| {
        workflows
            .iter()
            .find(|w| same_login(&w.owner, owner))
            .map(|w| w.workspace.as_str())
    };
    let part = &shown.observation.linear;
    let read_elsewhere: Vec<String> = part
        .viewers
        .iter()
        .map(|v| (&v.owner, &v.workspace))
        .chain(part.issues.iter().map(|i| (&i.owner, &i.id.workspace)))
        .filter(|(owner, workspace)| {
            !workspace_of(owner).is_some_and(|w| w.eq_ignore_ascii_case(workspace))
        })
        .map(|(owner, _)| owner.clone())
        .collect();
    let keep = |owner: &str| {
        workspace_of(owner).is_some() && !read_elsewhere.iter().any(|o| same_login(o, owner))
    };
    let part = &mut shown.observation.linear;
    part.viewers.retain(|v| keep(&v.owner));
    part.issues.retain(|i| keep(&i.owner));
    let kept = |source: &Source| match source {
        Source::Owner { owner, .. } if source.is_linear() => keep(owner),
        _ => true,
    };
    shown.observation.coverage.retain(|e| kept(&e.source));
    shown.carried.retain(|c| kept(&c.source));
}

/// Drops the Linear part, its coverage, and any age carried for a Linear
/// source (until #34 merges Linear per source, den never keeps Linear facts
/// across surveys, so no Linear data is older than its survey).
fn without_linear(shown: &mut CachedObservation) {
    shown.observation.linear = Default::default();
    shown.observation.coverage.retain(|e| !e.source.is_linear());
    shown.carried.retain(|c| !c.source.is_linear());
}

fn linear_log(id: SurveyId, survey: &LinearSurvey) -> Effect {
    let incomplete: Vec<String> = survey
        .coverage
        .incomplete()
        .map(|e| format!("{} {}", e.source.label(), e.state.describe()))
        .collect();
    Effect::log(
        if incomplete.is_empty() {
            LogLevel::Info
        } else {
            LogLevel::Warn
        },
        format!(
            "Linear survey {} finished: {} Linear Issue(s), {} Linear viewer(s); incomplete: [{}]",
            id.0,
            survey.part.issues.len(),
            survey.part.viewers.len(),
            incomplete.join("; ")
        ),
    )
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
