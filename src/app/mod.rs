//! The App (test seam 2): `handle(Event) -> (ViewModel, Effects)`.
//!
//! The App holds den's state and decides what to show and what to ask the ports
//! for. It does no I/O and reads no clock: time arrives in events.

mod editor;
mod effect;
mod event;
mod launch;
mod linear_writes;
mod overlay;
mod release;
mod scope;
mod upstream;
mod view;
pub mod view_model;

pub use effect::{Effect, LogLevel, LogRecord};
pub use event::{Event, Key, LaunchId, SurveyId};
pub use scope::Focus;
pub use view_model::ViewModel;

use crate::{
    cache::{self, CacheError, CachedObservation, LinearReads},
    config::{Config, ConfigError},
    decide::{self, Decision},
    domain::ids::same_login,
    domain::{
        Coverage, LinearSourceKind, Observation, Policies, Source, SourceKind, SourceState,
        UpstreamTargets, Viewer, Viewers, WorkItemId,
        facts::{MemberStateAttempt, PullRequestRef},
    },
    handoff::{
        JobCheck, JobsCheck,
        prompts::{Prompts, PromptsError},
    },
    layout::{Layout, TerminalSize},
    ordering,
    ports::{
        LinearSurvey, LinearSurveyRequest, MemberStatesRead, MemberStatesRequest, SurveyFailure,
        SurveyRequest, SurveyTarget,
    },
    start::Start,
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
    /// The Mark for release sheet (#38).
    Release(Box<release::ReleaseSheet>),
}

/// A survey answer held until the startup cache read answers.
struct HeldAnswer {
    id: SurveyId,
    outcome: Result<Box<Observation>, SurveyFailure>,
    now: Timestamp,
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
    /// The startup cache read is queued and hasn't answered. The Worker reads
    /// the cache and surveys on different lanes, so a survey can answer first.
    awaiting_cache: bool,
    /// A survey answer that arrived while `awaiting_cache`: it is merged with
    /// the cache once that is read, so data kept for failed sources survives.
    held_answer: Option<HeldAnswer>,
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
    /// The user's Claude directory (`$CLAUDE_CONFIG_DIR`, else `~/.claude`),
    /// read by each launch sheet's workflow availability check (#41).
    claude_dir: Option<PathBuf>,
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
    /// What the start rules had to say about where den opened (#90), until
    /// the user chooses another Scope.
    start_note: Option<String>,
    /// Each Owner's Linear Workflow policy for the decision engine, from the
    /// config: the empty policy for an Owner without a Linear Workflow.
    policies: Policies,
    /// The environment variables every Job runs without (#70, ADR-0007):
    /// the config's Linear key variables, sorted; empty without a config.
    removed_env: Vec<String>,
    /// The Linear survey in flight, if any (#32).
    linear_running: Option<SurveyId>,
    /// A Linear answer that arrived while nothing was shown (before the cache
    /// loaded and before GitHub answered): merged into the first Observation
    /// den shows, then gone. Every other Linear answer is merged into what is
    /// shown at once, and stays in it (a GitHub survey keeps the Linear part
    /// as it was).
    unshown_linear: Option<LinearSurvey>,
    /// The read of Landing Group members' states in flight, if any (#37),
    /// by the survey round it follows.
    member_states_running: Option<SurveyId>,
    /// A Linear answer was shown but not yet saved: the GitHub answer of the
    /// same survey was still to come, and saves both (or, if it fails, the
    /// failure does).
    linear_unsaved: bool,
    /// What the last `m` did (#77), and for which Work Item.
    copied: Option<(WorkItemId, Result<(), String>)>,
}

impl App {
    pub fn new(config: Result<Config, ConfigError>) -> Self {
        let policies = config.as_ref().map(Config::policies).unwrap_or_default();
        let removed_env = config
            .as_ref()
            .map(Config::job_removed_env)
            .unwrap_or_default();
        Self {
            config,
            running: None,
            next_survey: 0,
            shown: None,
            from_cache: false,
            awaiting_cache: false,
            held_answer: None,
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
            claude_dir: None,
            next_launch: 0,
            job_checks: Vec::new(),
            upstreams: UpstreamTargets::default(),
            policies,
            removed_env,
            linear_running: None,
            unshown_linear: None,
            member_states_running: None,
            linear_unsaved: false,
            upstreams_unreadable: None,
            upstream_problem: None,
            copied: None,
            start_note: None,
        }
    }

    /// Opens where the start rules said (#90). A watched Owner's Repo opens in
    /// its Repo Scope. A Repo outside the watched Owners can't be surveyed
    /// until the Repo Survey exists (milestone 3, Plan 2), so den opens All
    /// Owners and says so. No Repo chosen opens All Owners, with the rules'
    /// notice if they gave one.
    #[must_use]
    pub fn with_start(mut self, start: Start) -> Self {
        match start {
            Start::Repo(repo) => {
                if self.owners().iter().any(|o| same_login(o, &repo.owner)) {
                    let scope = ordering::Scope::Repo(repo);
                    self.scope = ScopeState {
                        cursor: scope.clone(),
                        scope,
                        ..ScopeState::default()
                    };
                } else {
                    self.start_note = Some(format!(
                        "{repo} is not a watched Owner's Repo, so den can't survey it yet and opened All Owners"
                    ));
                }
            }
            Start::Unchosen { notice } => self.start_note = notice,
        }
        self
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

    /// Sets the user's Claude directory, whose commands and skills each launch
    /// sheet checks its prompt's slash commands against (#41); `None`: den
    /// doesn't know it, and the sheet says so.
    #[must_use]
    pub fn with_claude_dir(mut self, claude_dir: Option<PathBuf>) -> Self {
        self.claude_dir = claude_dir;
        self
    }

    pub fn handle(&mut self, event: Event) -> (ViewModel, Vec<Effect>) {
        let mut effects = match event {
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
            Event::MemberStatesRead { id, read, now } => {
                self.now = Some(now);
                self.member_states_read(id, *read, now)
            }
            Event::CacheLoaded { outcome } => self.cache_loaded(outcome),
            Event::CacheStored { outcome } => cache_stored(outcome),
            Event::LaunchInspected { id, view } => self.launch_inspected(id, view),
            Event::LaunchFinished { id, report } => self.launch_finished(id, report),
            Event::LinearIssueRead { id, read } => self.linear_issue_read(id, *read),
            Event::LinearWriteFinished { id, write, outcome } => {
                self.linear_write_finished(id, write, *outcome)
            }
            Event::JobsChecked { outcome } => self.jobs_checked(outcome),
            Event::UpstreamsStored { outcome } => self.upstreams_stored(outcome),
            Event::Copied { item, outcome } => self.copy_finished(item, outcome),
        };
        effects.extend(self.finish_land_check());
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
            legacy_marker_prefixes: self.policies.legacy_marker_prefixes(),
            copied: self.copied.as_ref(),
            start_note: self.start_note.as_deref(),
        })
    }

    fn start(&mut self) -> Vec<Effect> {
        match &self.config {
            Ok(config) => {
                let request = SurveyRequest {
                    target: SurveyTarget::Owners(config.owners.clone()),
                    upstreams: self.upstreams.clone(),
                    screenshot_repos: config.screenshot_repos.clone(),
                };
                // The cache is read first, so it shows before the survey answers;
                // then the Jobs (local and bounded), so their markers don't wait
                // for gh on the Worker's remote lane.
                let mut effects = vec![Effect::LoadCache, Effect::CheckJobs];
                self.awaiting_cache = true;
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
            Effect::ReadLinearIssue { id, .. }
            | Effect::AssignLinearIssue { id, .. }
            | Effect::MoveLinearIssue { id, .. } => {
                let mut effects = vec![Effect::log(
                    LogLevel::Error,
                    format!(
                        "could not queue a Linear request for sheet {}: {reason}",
                        id.0
                    ),
                )];
                effects.extend(self.linear_not_queued(id, &effect, &reason));
                return effects;
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
            Effect::LoadCache => {
                let mut effects = vec![Effect::log(
                    LogLevel::Error,
                    format!("could not queue read cache: {reason}"),
                )];
                effects.extend(self.cache_answered());
                return effects;
            }
            Effect::ReadMemberStates { id, .. } => {
                if self.member_states_running == Some(id) {
                    self.member_states_running = None;
                }
                "read Landing Group members' states"
            }
            Effect::CopyToClipboard { item, .. } => {
                return self.copy_finished(item, Err(format!("not queued: {reason}")));
            }
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
        self.linear_answered(id, survey, self.now)
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
            format!("survey {} started for {}", id.0, request.target),
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
        // So does the Mark for release sheet (#38).
        if let Some(OverlayState::Release(_)) = &self.overlay
            && let Some(OverlayState::Release(sheet)) = self.overlay.take()
        {
            return self.release_key(sheet, key);
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
                // The start notice was about where den opened.
                self.start_note = None;
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
            Key::Char('m') => self.copy_markers(),
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
        if self.awaiting_cache {
            // Still `running`, so the view says it is refreshing and `r` waits.
            self.held_answer = Some(HeldAnswer { id, outcome, now });
            return vec![Effect::log(
                LogLevel::Info,
                format!(
                    "survey {} answered before the cache was read; holding it until then",
                    id.0
                ),
            )];
        }
        self.running = None;
        match outcome {
            Ok(observation) => {
                let mut effects = vec![survey_log(id, &observation)];
                // The merge keeps the Linear part shown, if any.
                let mut merged = cache::merge(self.shown.take(), *observation);
                // Nothing was shown: Linear's answer, if any, is still apart.
                if let Some(unshown) = self.unshown_linear.take() {
                    merged = self.merge_linear_logged(merged, unshown, &mut effects);
                }
                self.record_unanswered_linear(&mut merged.observation.coverage);
                // Linear's kept data was logged when its answer was merged
                // (nothing earlier was shown for an unshown answer to keep).
                effects.extend(carried_log(&merged, |source| !source.is_linear()));
                self.show(merged, now);
                self.from_cache = false;
                self.failure = None;
                self.linear_unsaved = false;
                let wanted = self.settle_member_states(now);
                if let Some(shown) = &self.shown {
                    effects.push(Effect::StoreCache {
                        cached: Box::new(shown.clone()),
                    });
                }
                effects.extend(self.read_member_states(id, wanted));
                effects
            }
            Err(failure) => {
                let log = Effect::log(
                    LogLevel::Error,
                    format!("survey {} failed: {failure}", id.0),
                );
                // The last good Observation, if any, stays on screen.
                let mut effects = vec![log];
                let wanted = self.settle_member_states(now);
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
                effects.extend(self.read_member_states(id, wanted));
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
                    target: SurveyTarget::Owners(config.owners.clone()),
                    upstreams: self.upstreams.clone(),
                    screenshot_repos: config.screenshot_repos.clone(),
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
        let mut effects = self.show_cache(outcome);
        effects.extend(self.cache_answered());
        effects
    }

    /// The startup cache read answered (or was never queued): handles the
    /// survey answer held for it, if any.
    fn cache_answered(&mut self) -> Vec<Effect> {
        self.awaiting_cache = false;
        match self.held_answer.take() {
            Some(HeldAnswer { id, outcome, now }) => {
                self.survey_finished(id, outcome, self.now.unwrap_or(now))
            }
            None => Vec::new(),
        }
    }

    fn show_cache(
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
        let mut cached = cached
            .restricted_to(&config.owners)
            .restricted_to_linear(&linear_reads(config));
        // shortcut: without cached query scopes, re-fetch; persist scopes to reuse startup coverage.
        cached.observation.coverage.retain(|entry| {
            !matches!(
                entry.source,
                Source::Owner {
                    kind: SourceKind::Linear(
                        LinearSourceKind::StartTeamIssues | LinearSourceKind::TeamStates
                    ),
                    ..
                }
            )
        });
        let mut effects = vec![Effect::log(
            LogLevel::Info,
            format!(
                "showing the cached Observation of {} from {}",
                cached.observation.viewer.login, cached.observation.observed_at
            ),
        )];
        if let Some(unshown) = self.unshown_linear.take() {
            cached = self.merge_linear_logged(cached, unshown, &mut effects);
            effects.extend(carried_log(&cached, Source::is_linear));
        }
        self.record_unanswered_linear(&mut cached.observation.coverage);
        let now = self.now.unwrap_or(cached.observation.observed_at);
        self.show(cached, now);
        self.from_cache = true;
        effects
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

    /// Narrows the list to the Scope, keeping the selection when it is still
    /// in it. Work Items whose only Next Step their Repo doesn't track
    /// (`skip`, spec 0003) are left out; the empty state counts them.
    fn apply_scope(&mut self) {
        self.decisions = ordering::in_scope(&self.ordered, &self.scope.scope)
            .filter(|d| d.not_tracked.is_none())
            .cloned()
            .collect();
        self.keep_selection();
    }

    /// `m` (#77): the selected PR's needs-decision Points' marker lines
    /// (answered) onto the clipboard, for the user to post with their
    /// decision. den posts nothing (ADR-0002). Nothing when no Point needs a
    /// decision.
    fn copy_markers(&mut self) -> Vec<Effect> {
        let Some(item) = self.selected.clone() else {
            return Vec::new();
        };
        let Some(text) = view::points::of(
            self.shown.as_ref().map(|s| &s.observation),
            self.policies.legacy_marker_prefixes(),
            &item,
        )
        .as_ref()
        .and_then(view::points::decision_markers) else {
            return Vec::new();
        };
        vec![Effect::CopyToClipboard { item, text }]
    }

    fn copy_finished(&mut self, item: WorkItemId, outcome: Result<(), String>) -> Vec<Effect> {
        let log = match &outcome {
            Ok(()) => Effect::log(
                LogLevel::Info,
                format!("copied the answering marker line(s) of {item} to the clipboard"),
            ),
            Err(why) => Effect::log(
                LogLevel::Warn,
                format!("couldn't copy the marker line(s) of {item}: {why}"),
            ),
        };
        self.copied = Some((item, outcome));
        vec![log]
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
    /// A Linear survey answered: it is merged into what is shown, Linear source
    /// by Linear source (a source read completely replaces its data, any other
    /// keeps the earlier data with its age).
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
        self.linear_answered(id, survey, Some(now))
    }

    /// Takes the Linear answer of survey `id` into what is shown, merged
    /// Linear source by Linear source, or keeps it apart while nothing is
    /// shown. It is saved at once unless the same survey's GitHub answer is
    /// still to come (that one saves both, so a refresh writes the cache once).
    /// `now` is when it arrived, if den knows the time yet.
    fn linear_answered(
        &mut self,
        id: SurveyId,
        mut survey: LinearSurvey,
        now: Option<Timestamp>,
    ) -> Vec<Effect> {
        // A Linear source that didn't date its answer: dated when it arrived.
        if let Some(now) = now {
            survey.part.observed_at.get_or_insert(now);
        }
        let mut effects = vec![linear_log(id, &survey)];
        let Some(shown) = self.shown.take() else {
            self.unshown_linear = Some(survey);
            // GitHub already failed with nothing shown: show Linear's answer.
            if self.running.is_none()
                && let Some(failure) = self.failure.clone()
                && let Some(now) = now
            {
                self.show_linear_only(&failure, now);
            }
            return effects;
        };
        let merged = self.merge_linear_logged(shown, survey, &mut effects);
        // GitHub's kept data was logged when its answer arrived.
        effects.extend(carried_log(&merged, Source::is_linear));
        let at = now.unwrap_or(merged.observation.observed_at);
        self.show(merged, at);
        let wanted = self.settle_member_states(at);
        match &self.shown {
            // The Linear-only stand-in: never saved.
            Some(shown) if !has_github_viewer(shown) => {}
            Some(_) if self.running == Some(id) => self.linear_unsaved = true,
            Some(shown) => effects.push(Effect::StoreCache {
                cached: Box::new(shown.clone()),
            }),
            None => {}
        }
        effects.extend(self.read_member_states(id, wanted));
        effects
    }

    /// Once neither survey of the round is still running: settles the
    /// members' states against the Landing Groups just decided
    /// ([`cache::settle_member_states`]) and returns the members whose state
    /// den should read, unread ones first. Empty while a survey of the
    /// round is still out.
    fn settle_member_states(&mut self, now: Timestamp) -> Vec<PullRequestRef> {
        if self.running.is_some() || self.linear_running.is_some() {
            return Vec::new();
        }
        let Some(shown) = self.shown.take() else {
            return Vec::new();
        };
        let (members, wanted) = landing_members(&self.ordered, &shown.observation.member_attempts);
        let size = |c: &CachedObservation| {
            (
                c.observation.member_states.len(),
                c.observation.coverage.entries().len(),
            )
        };
        let before = size(&shown);
        let settled = cache::settle_member_states(shown, &members, &wanted);
        if size(&settled) == before {
            self.shown = Some(settled);
        } else {
            self.show(settled, now);
        }
        wanted
    }

    /// After round `id`: asks for the states of `pull_requests`, the Landing
    /// Group members that weren't among the open Pull Requests and aren't
    /// known merged (#37), if there are any. Merged is final, so a member
    /// read merged is never asked about again; one read closed is, in case
    /// it was reopened and merged since.
    fn read_member_states(
        &mut self,
        id: SurveyId,
        pull_requests: Vec<PullRequestRef>,
    ) -> Vec<Effect> {
        if pull_requests.is_empty() {
            return Vec::new();
        }
        self.member_states_running = Some(id);
        vec![
            Effect::log(
                LogLevel::Info,
                format!(
                    "reading the states of {} Landing Group member(s) not among the open Pull Requests",
                    pull_requests.len()
                ),
            ),
            Effect::ReadMemberStates {
                id,
                request: MemberStatesRequest { pull_requests },
            },
        ]
    }

    /// Landing Group members' states arrived: merged into what is shown,
    /// decided again and cached. An answer for an older round is ignored.
    fn member_states_read(
        &mut self,
        id: SurveyId,
        read: MemberStatesRead,
        now: Timestamp,
    ) -> Vec<Effect> {
        if self.member_states_running != Some(id) {
            return vec![Effect::log(
                LogLevel::Warn,
                format!(
                    "ignored the members' states read after survey {}, which is no longer current",
                    id.0
                ),
            )];
        }
        self.member_states_running = None;
        let incomplete: Vec<String> = read
            .coverage
            .incomplete()
            .map(|e| format!("{} {}", e.source.label(), e.state.describe()))
            .collect();
        let mut effects = vec![Effect::log(
            if incomplete.is_empty() {
                LogLevel::Info
            } else {
                LogLevel::Warn
            },
            format!(
                "read {} Landing Group member state(s) after survey {}; incomplete: [{}]",
                read.states.len(),
                id.0,
                incomplete.join("; ")
            ),
        )];
        let Some(shown) = self.shown.take() else {
            return effects;
        };
        let merged = cache::merge_member_states(shown, read, now);
        self.show(merged.clone(), now);
        // The Linear-only stand-in is never saved (no GitHub viewer).
        if has_github_viewer(&merged) {
            effects.push(Effect::StoreCache {
                cached: Box::new(merged),
            });
        }
        effects
    }

    /// With no GitHub answer to show, shows the unshown Linear answer alone,
    /// if it holds Linear Issues. GitHub's viewer is unknown and its failure
    /// is coverage, so nothing reads as complete. It is never saved (the
    /// cache is per GitHub viewer). Later Linear answers merge into it, and
    /// the next GitHub answer keeps its Linear part, kept data and ages
    /// included, as any GitHub answer does.
    fn show_linear_only(&mut self, failure: &SurveyFailure, now: Timestamp) {
        let Ok(config) = &self.config else {
            return;
        };
        let Some(unshown) = self.unshown_linear.take_if(|l| !l.part.issues.is_empty()) else {
            return;
        };
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
        // Nothing earlier to merge into, so no Linear viewer can have changed;
        // every Linear source the config reads is recorded by the merge.
        let shown = cache::merge_linear(
            CachedObservation::fresh(observation),
            unshown,
            &linear_reads(config),
        )
        .cached;
        self.show(shown, now);
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

    /// Every Linear source of every configured Linear Workflow
    /// ([`LinearWorkflow::linear_sources`](crate::config::LinearWorkflow::linear_sources)
    /// per workflow).
    fn linear_sources(&self) -> Vec<Source> {
        let Ok(config) = &self.config else {
            return Vec::new();
        };
        config
            .linear
            .iter()
            .flat_map(|workflow| {
                workflow
                    .linear_sources()
                    .into_iter()
                    .map(|kind| Source::Owner {
                        owner: workflow.owner.clone(),
                        kind: SourceKind::Linear(kind),
                    })
            })
            .collect()
    }

    /// Merges `survey` into `cached` for the configured Linear Workflows,
    /// logging any Owner whose earlier Linear data was dropped because its
    /// Linear viewer changed.
    fn merge_linear_logged(
        &self,
        cached: CachedObservation,
        survey: LinearSurvey,
        effects: &mut Vec<Effect>,
    ) -> CachedObservation {
        let reads = self.config.as_ref().map(linear_reads).unwrap_or_default();
        let merge = cache::merge_linear(cached, survey, &reads);
        if !merge.viewer_changed.is_empty() {
            effects.push(Effect::log(
                LogLevel::Warn,
                format!(
                    "the Linear viewer of {} is not the one den's Linear data was read as; dropped that data",
                    merge.viewer_changed.join(", ")
                ),
            ));
        }
        merge.cached
    }
}

/// What den reads from Linear, per configured Linear Workflow.
fn linear_reads(config: &Config) -> Vec<LinearReads<'_>> {
    config
        .linear
        .iter()
        .map(|w| LinearReads {
            owner: &w.owner,
            workspace: &w.workspace,
            sources: w.linear_sources(),
        })
        .collect()
}

/// Whether `shown` holds a GitHub answer (or a cached one): only then does
/// it have a GitHub viewer to be saved under. GitHub logins are never
/// empty; the Linear-only stand-in's is.
fn has_github_viewer(shown: &CachedObservation) -> bool {
    !shown.observation.viewer.login.is_empty()
}

/// From the Linear Issues' Decisions (their Landing Groups, already
/// correlated while deciding): every member, and the members whose state den
/// should read, each once. Those are attached, not among their Repo's open
/// Pull Requests (which den read completely), and not known merged.
///
/// The read is bounded, so they come in this order: members den never asked
/// about; then members asked about and not answered (`attempts`), the
/// longest unanswered first, so a member that never resolves moves behind
/// the others each round instead of holding its place; then open again;
/// then read closed. Members re-read every round never keep one den hasn't
/// asked about yet beyond the bound.
fn landing_members(
    decisions: &[Decision],
    attempts: &[MemberStateAttempt],
) -> (Vec<PullRequestRef>, Vec<PullRequestRef>) {
    use decide::correlation::{MemberStatus, PullRequestKey};
    use std::collections::HashSet;
    let mut members: Vec<PullRequestRef> = Vec::new();
    let mut seen: HashSet<PullRequestKey> = HashSet::new();
    let mut queued: HashSet<PullRequestKey> = HashSet::new();
    let mut never_asked = Vec::new();
    let mut unanswered: Vec<(Timestamp, PullRequestRef)> = Vec::new();
    let (mut open_again, mut closed) = (Vec::new(), Vec::new());
    for member in decisions.iter().flat_map(|d| &d.landing.members) {
        let key = PullRequestKey::of(&member.pr.repo, member.pr.number);
        if seen.insert(key.clone()) {
            members.push(member.pr.clone());
        }
        let wanted = matches!(
            member.status,
            MemberStatus::NotOpen | MemberStatus::OpenAgain | MemberStatus::Closed
        );
        if !wanted || !queued.insert(key) {
            continue;
        }
        let pr = member.pr.clone();
        match member.status {
            MemberStatus::NotOpen => match attempts.iter().find(|a| a.pr.is(&pr.repo, pr.number)) {
                Some(attempt) => unanswered.push((attempt.attempted_at, pr)),
                None => never_asked.push(pr),
            },
            MemberStatus::OpenAgain => open_again.push(pr),
            _ => closed.push(pr),
        }
    }
    // Stable: members asked at the same time keep their group order.
    unanswered.sort_by_key(|(attempted_at, _)| *attempted_at);
    let mut wanted = never_asked;
    wanted.extend(unanswered.into_iter().map(|(_, pr)| pr));
    wanted.extend(open_again);
    wanted.extend(closed);
    (members, wanted)
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

/// Logs which of the sources `of` picks show data kept from an earlier
/// survey: a survey of one service logs its own.
fn carried_log(merged: &CachedObservation, of: impl Fn(&Source) -> bool) -> Option<Effect> {
    let kept: Vec<String> = merged
        .carried
        .iter()
        .filter(|c| of(&c.source))
        .map(|c| format!("{} from {}", c.source.label(), c.observed_at))
        .collect();
    if kept.is_empty() {
        return None;
    }
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
