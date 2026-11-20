//! Fake ports for seam-2 tests, and a scripted GraphQL transport for the GitHub
//! adapter tests. `FakeWorkspace` behaves like git just enough for the launch
//! executor (fetch fills tracking refs, adding a worktree makes its path exist);
//! `FakeLauncher` and `FakeJobs` record what they were asked to do.

use den::{
    cache::{CacheError, CachedObservation},
    domain::Observation,
    github::graphql::{GraphQl, GraphQlRequest, GraphQlResponse, TransportError},
    handoff::{PathState, RegisteredWorktree, Remote, WindowCommand, WindowMode, Writability},
    jobs::{JobRecord, JobStoreError},
    ports::{
        Clock, GitHubSource, JobStore, LaunchError, Launcher, ObservationCache, Ports,
        SurveyFailure, SurveyRequest, Workspace, WorkspaceError, WorktreeAdd, WorktreeStart,
    },
};
use jiff::Timestamp;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type SurveyOutcome = Result<Observation, SurveyFailure>;

/// Answers surveys from a queue, and records every request.
#[derive(Default)]
pub struct FakeGitHub {
    answers: Mutex<VecDeque<SurveyOutcome>>,
    requests: Mutex<Vec<SurveyRequest>>,
}

impl FakeGitHub {
    pub fn answering(answers: impl IntoIterator<Item = SurveyOutcome>) -> Self {
        Self {
            answers: Mutex::new(answers.into_iter().collect()),
            requests: Mutex::default(),
        }
    }

    pub fn push_answer(&self, answer: SurveyOutcome) {
        self.answers.lock().expect("lock").push_back(answer);
    }

    pub fn requests(&self) -> Vec<SurveyRequest> {
        self.requests.lock().expect("lock").clone()
    }
}

impl GitHubSource for FakeGitHub {
    fn survey(&self, request: &SurveyRequest) -> SurveyOutcome {
        self.requests.lock().expect("lock").push(request.clone());
        self.answers
            .lock()
            .expect("lock")
            .pop_front()
            .unwrap_or_else(|| {
                Err(SurveyFailure::Interrupted {
                    detail: "the fake GitHub source has no more answers".to_owned(),
                })
            })
    }
}

/// One fake clone: its remotes, local refs (`refs/heads/…`, `refs/remotes/…`),
/// and what its remotes' branches point at right now.
#[derive(Debug, Clone, Default)]
pub struct FakeClone {
    pub remotes: Vec<Remote>,
    pub refs: BTreeMap<String, String>,
    /// `(remote, branch)` → commit, as `git ls-remote` would answer.
    pub remote_heads: BTreeMap<(String, String), String>,
    pub not_writable: bool,
    /// Worktrees registered for the clone (besides its own checkout).
    pub worktrees: Vec<RegisteredWorktree>,
}

/// What the Workspace was asked to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCall {
    Fetch {
        clone: PathBuf,
        remote: String,
        refspec: String,
    },
    AddWorktree {
        clone: PathBuf,
        add: WorktreeAdd,
    },
    Clone {
        url: String,
        remote: String,
        into: PathBuf,
    },
}

#[derive(Default)]
struct WorkspaceState {
    clones: BTreeMap<PathBuf, FakeClone>,
    /// Other paths that exist (worktrees added, folders in the way).
    existing: BTreeSet<PathBuf>,
    /// Operation name ("fetch", "add_worktree", "ls-remote", "clone") → error.
    failures: BTreeMap<String, String>,
    /// Paths den can't inspect, with the error.
    unknown: BTreeMap<PathBuf, String>,
    /// Folders (not clones) den may not write in.
    unwritable: BTreeSet<PathBuf>,
    /// Repos reachable by URL (not cloned): URL → branch → commit.
    hosted: BTreeMap<String, BTreeMap<String, String>>,
    calls: Vec<WorkspaceCall>,
}

/// An in-memory workspace.
#[derive(Default)]
pub struct FakeWorkspace {
    state: Mutex<WorkspaceState>,
}

impl FakeWorkspace {
    /// Adds a clone at `path` with `(name, url)` remotes.
    pub fn add_clone(&self, path: &str, remotes: &[(&str, &str)]) {
        let clone = FakeClone {
            remotes: remotes
                .iter()
                .map(|(name, url)| Remote {
                    name: (*name).to_owned(),
                    url: (*url).to_owned(),
                })
                .collect(),
            ..FakeClone::default()
        };
        self.state
            .lock()
            .expect("lock")
            .clones
            .insert(path.into(), clone);
    }

    fn with_clone(&self, path: &str, change: impl FnOnce(&mut FakeClone)) {
        let mut state = self.state.lock().expect("lock");
        change(
            state
                .clones
                .get_mut(Path::new(path))
                .expect("a fake clone at that path"),
        );
    }

    /// The Repo at `url` (reachable, not cloned) has `branch` at `commit`.
    pub fn host(&self, url: &str, branch: &str, commit: &str) {
        self.state
            .lock()
            .expect("lock")
            .hosted
            .entry(url.to_owned())
            .or_default()
            .insert(branch.to_owned(), commit.to_owned());
    }

    /// `branch` on `remote` points at `commit` (what `ls-remote` answers and `fetch` brings).
    pub fn set_remote_head(&self, clone: &str, remote: &str, branch: &str, commit: &str) {
        self.with_clone(clone, |c| {
            c.remote_heads
                .insert((remote.to_owned(), branch.to_owned()), commit.to_owned());
        });
    }

    /// A local branch `branch` at `commit`.
    pub fn set_local_branch(&self, clone: &str, branch: &str, commit: &str) {
        self.with_clone(clone, |c| {
            c.refs
                .insert(format!("refs/heads/{branch}"), commit.to_owned());
        });
    }

    /// `branch` is checked out in a worktree at `path` (registered with git).
    pub fn register_worktree(&self, clone: &str, path: &str, branch: Option<&str>) {
        self.with_clone(clone, |c| {
            c.worktrees.push(RegisteredWorktree {
                path: path.into(),
                branch: branch.map(str::to_owned),
                locked: false,
                prunable: false,
            });
        });
    }

    pub fn make_unwritable(&self, clone: &str) {
        self.with_clone(clone, |c| c.not_writable = true);
    }

    /// den can't inspect `path`.
    pub fn make_unknown(&self, path: &str, error: &str) {
        self.state
            .lock()
            .expect("lock")
            .unknown
            .insert(path.into(), error.to_owned());
    }

    /// A folder (not a clone) at `path` that den may not write in.
    pub fn add_unwritable_folder(&self, path: &str) {
        let mut state = self.state.lock().expect("lock");
        state.existing.insert(path.into());
        state.unwritable.insert(path.into());
    }

    /// Something already exists at `path`.
    pub fn add_existing(&self, path: &str) {
        self.state
            .lock()
            .expect("lock")
            .existing
            .insert(path.into());
    }

    /// The next `operation` ("fetch", "add_worktree", "ls-remote") fails.
    pub fn fail(&self, operation: &str, error: &str) {
        self.state
            .lock()
            .expect("lock")
            .failures
            .insert(operation.to_owned(), error.to_owned());
    }

    /// Every change the Workspace was asked for, in order.
    pub fn calls(&self) -> Vec<WorkspaceCall> {
        self.state.lock().expect("lock").calls.clone()
    }

    fn failure(state: &WorkspaceState, operation: &str) -> Result<(), WorkspaceError> {
        match state.failures.get(operation) {
            Some(detail) => Err(WorkspaceError {
                operation: operation.to_owned(),
                detail: detail.clone(),
            }),
            None => Ok(()),
        }
    }
}

fn no_clone(path: &Path) -> WorkspaceError {
    WorkspaceError {
        operation: format!("open {}", path.display()),
        detail: "not a git repository".to_owned(),
    }
}

impl Workspace for FakeWorkspace {
    fn find_clones(&self, root: &Path) -> Result<Vec<PathBuf>, WorkspaceError> {
        let state = self.state.lock().expect("lock");
        Ok(state
            .clones
            .keys()
            .filter(|p| p.starts_with(root))
            .cloned()
            .collect())
    }

    fn remotes(&self, clone: &Path) -> Result<Vec<Remote>, WorkspaceError> {
        let state = self.state.lock().expect("lock");
        state
            .clones
            .get(clone)
            .map(|c| c.remotes.clone())
            .ok_or_else(|| no_clone(clone))
    }

    fn path_state(&self, path: &Path) -> PathState {
        let state = self.state.lock().expect("lock");
        if let Some(error) = state.unknown.get(path) {
            return PathState::Unknown(error.clone());
        }
        // Folders holding a clone or an existing path exist too.
        let holds_something = state
            .clones
            .keys()
            .chain(state.existing.iter())
            .any(|p| p.starts_with(path));
        match state.clones.get(path) {
            Some(clone) => PathState::Exists {
                writable: if clone.not_writable {
                    Writability::NotWritable
                } else {
                    Writability::Writable
                },
            },
            None if state.existing.contains(path) || holds_something => PathState::Exists {
                writable: if state.unwritable.contains(path) {
                    Writability::NotWritable
                } else {
                    Writability::Writable
                },
            },
            None => PathState::Missing,
        }
    }

    fn resolve(&self, clone: &Path, reference: &str) -> Result<Option<String>, WorkspaceError> {
        let state = self.state.lock().expect("lock");
        let clone = state.clones.get(clone).ok_or_else(|| no_clone(clone))?;
        Ok(clone.refs.get(reference).cloned())
    }

    fn remote_branch_head(
        &self,
        clone: &Path,
        remote: &str,
        branch: &str,
    ) -> Result<Option<String>, WorkspaceError> {
        let state = self.state.lock().expect("lock");
        Self::failure(&state, "ls-remote")?;
        let clone = state.clones.get(clone).ok_or_else(|| no_clone(clone))?;
        Ok(clone
            .remote_heads
            .get(&(remote.to_owned(), branch.to_owned()))
            .cloned())
    }

    fn worktrees(&self, clone: &Path) -> Result<Vec<RegisteredWorktree>, WorkspaceError> {
        let state = self.state.lock().expect("lock");
        let fake = state.clones.get(clone).ok_or_else(|| no_clone(clone))?;
        Ok(fake.worktrees.clone())
    }

    fn fetch(&self, clone: &Path, remote: &str, refspec: &str) -> Result<(), WorkspaceError> {
        let mut state = self.state.lock().expect("lock");
        state.calls.push(WorkspaceCall::Fetch {
            clone: clone.to_owned(),
            remote: remote.to_owned(),
            refspec: refspec.to_owned(),
        });
        Self::failure(&state, "fetch")?;
        let fake = state.clones.get_mut(clone).ok_or_else(|| no_clone(clone))?;
        // `+refs/heads/<b>:refs/remotes/<r>/<b>`
        let (source, destination) = refspec
            .trim_start_matches('+')
            .split_once(':')
            .expect("an explicit refspec");
        let branch = source.trim_start_matches("refs/heads/");
        if let Some(commit) = fake
            .remote_heads
            .get(&(remote.to_owned(), branch.to_owned()))
            .cloned()
        {
            fake.refs.insert(destination.to_owned(), commit);
        }
        Ok(())
    }

    fn add_worktree(&self, clone: &Path, add: &WorktreeAdd) -> Result<(), WorkspaceError> {
        let mut state = self.state.lock().expect("lock");
        state.calls.push(WorkspaceCall::AddWorktree {
            clone: clone.to_owned(),
            add: add.clone(),
        });
        Self::failure(&state, "add_worktree")?;
        let fake = state.clones.get_mut(clone).ok_or_else(|| no_clone(clone))?;
        if let WorktreeStart::NewBranchTracking { start } | WorktreeStart::NewBranch { start } =
            &add.start
        {
            let commit = fake.refs.get(start).cloned().expect("the start ref exists");
            fake.refs
                .insert(format!("refs/heads/{}", add.branch), commit);
        }
        fake.worktrees.push(RegisteredWorktree {
            path: add.path.clone(),
            branch: Some(add.branch.clone()),
            locked: false,
            prunable: false,
        });
        state.existing.insert(add.path.clone());
        Ok(())
    }

    fn url_branch_head(&self, url: &str, branch: &str) -> Result<Option<String>, WorkspaceError> {
        let state = self.state.lock().expect("lock");
        Self::failure(&state, "ls-remote")?;
        let hosted = state.hosted.get(url).ok_or_else(|| WorkspaceError {
            operation: format!("ask {url} for {branch}"),
            detail: "repository not found".to_owned(),
        })?;
        Ok(hosted.get(branch).cloned())
    }

    /// Makes a clone at `into` whose remote `remote` is `url`, with the
    /// hosted Repo's branches as remote-tracking refs.
    fn clone_repo(&self, url: &str, remote: &str, into: &Path) -> Result<(), WorkspaceError> {
        let mut state = self.state.lock().expect("lock");
        state.calls.push(WorkspaceCall::Clone {
            url: url.to_owned(),
            remote: remote.to_owned(),
            into: into.to_owned(),
        });
        Self::failure(&state, "clone")?;
        if state.clones.contains_key(into) || state.existing.contains(into) {
            return Err(WorkspaceError {
                operation: format!("clone into {}", into.display()),
                detail: "something already exists there".to_owned(),
            });
        }
        let branches = state.hosted.get(url).cloned().unwrap_or_default();
        let clone = FakeClone {
            remotes: vec![Remote {
                name: remote.to_owned(),
                url: url.to_owned(),
            }],
            refs: branches
                .iter()
                .map(|(b, c)| (format!("refs/remotes/{remote}/{b}"), c.clone()))
                .collect(),
            remote_heads: branches
                .iter()
                .map(|(b, c)| ((remote.to_owned(), b.clone()), c.clone()))
                .collect(),
            ..FakeClone::default()
        };
        state.clones.insert(into.to_owned(), clone);
        Ok(())
    }
}

/// A launcher that records what it would open or copy.
pub struct FakeLauncher {
    mode: Mutex<WindowMode>,
    claude: Mutex<bool>,
    windows: Mutex<Vec<WindowCommand>>,
    clipboard: Mutex<Vec<String>>,
    window_error: Mutex<Option<String>>,
}

impl Default for FakeLauncher {
    /// In kitty, with Claude Code on PATH.
    fn default() -> Self {
        Self {
            mode: Mutex::new(WindowMode::Kitty),
            claude: Mutex::new(true),
            windows: Mutex::default(),
            clipboard: Mutex::default(),
            window_error: Mutex::default(),
        }
    }
}

impl FakeLauncher {
    pub fn outside_kitty(&self) {
        *self.mode.lock().expect("lock") = WindowMode::Clipboard {
            why: "den is not running in kitty".to_owned(),
        };
    }

    pub fn without_claude(&self) {
        *self.claude.lock().expect("lock") = false;
    }

    pub fn fail_window(&self, error: &str) {
        *self.window_error.lock().expect("lock") = Some(error.to_owned());
    }

    pub fn windows(&self) -> Vec<WindowCommand> {
        self.windows.lock().expect("lock").clone()
    }

    pub fn clipboard(&self) -> Vec<String> {
        self.clipboard.lock().expect("lock").clone()
    }
}

impl Launcher for FakeLauncher {
    fn window_mode(&self) -> WindowMode {
        self.mode.lock().expect("lock").clone()
    }

    fn claude_available(&self) -> bool {
        *self.claude.lock().expect("lock")
    }

    fn open_window(&self, window: &WindowCommand) -> Result<(), LaunchError> {
        if let Some(detail) = self.window_error.lock().expect("lock").clone() {
            return Err(LaunchError {
                operation: "open a kitty window".to_owned(),
                detail,
            });
        }
        self.windows.lock().expect("lock").push(window.clone());
        Ok(())
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<(), LaunchError> {
        self.clipboard.lock().expect("lock").push(text.to_owned());
        Ok(())
    }
}

/// In-memory Job records.
#[derive(Default)]
pub struct FakeJobs {
    records: Mutex<Vec<JobRecord>>,
    load_error: Mutex<Option<JobStoreError>>,
}

impl FakeJobs {
    pub fn records(&self) -> Vec<JobRecord> {
        self.records.lock().expect("lock").clone()
    }

    pub fn holding(&self, job: JobRecord) {
        self.records.lock().expect("lock").push(job);
    }

    pub fn fail_load(&self, error: JobStoreError) {
        *self.load_error.lock().expect("lock") = Some(error);
    }
}

impl JobStore for FakeJobs {
    fn load(&self) -> Result<Vec<JobRecord>, JobStoreError> {
        match self.load_error.lock().expect("lock").clone() {
            Some(error) => Err(error),
            None => Ok(self.records()),
        }
    }

    fn record(&self, job: &JobRecord) -> Result<(), JobStoreError> {
        self.records.lock().expect("lock").push(job.clone());
        Ok(())
    }
}

/// A clock that always says the same time.
pub struct FixedClock(pub Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// An in-memory cache: `load` answers with what was last stored (or a scripted
/// answer), `store` records every write.
#[derive(Default)]
pub struct FakeCache {
    held: Mutex<Option<CachedObservation>>,
    load_answer: Mutex<Option<Result<Option<CachedObservation>, CacheError>>>,
    stored: Mutex<Vec<CachedObservation>>,
}

impl FakeCache {
    /// A cache already holding `cached`.
    pub fn holding(cached: CachedObservation) -> Self {
        let cache = Self::default();
        *cache.held.lock().expect("lock") = Some(cached);
        cache
    }

    /// The next `load` answers with `answer` instead of what is held.
    pub fn answer_load(&self, answer: Result<Option<CachedObservation>, CacheError>) {
        *self.load_answer.lock().expect("lock") = Some(answer);
    }

    /// Every Observation stored, in order.
    pub fn stored(&self) -> Vec<CachedObservation> {
        self.stored.lock().expect("lock").clone()
    }
}

impl ObservationCache for FakeCache {
    fn load(&self) -> Result<Option<CachedObservation>, CacheError> {
        match self.load_answer.lock().expect("lock").take() {
            Some(answer) => answer,
            None => Ok(self.held.lock().expect("lock").clone()),
        }
    }

    fn store(&self, cached: &CachedObservation) -> Result<(), CacheError> {
        self.stored.lock().expect("lock").push(cached.clone());
        *self.held.lock().expect("lock") = Some(cached.clone());
        Ok(())
    }
}

pub fn ports(github: Arc<FakeGitHub>, now: Timestamp) -> Ports {
    ports_with_cache(github, Arc::new(FakeCache::default()), now)
}

pub fn ports_with_cache(github: Arc<FakeGitHub>, cache: Arc<FakeCache>, now: Timestamp) -> Ports {
    Ports {
        github,
        cache,
        workspace: Arc::new(FakeWorkspace::default()),
        launcher: Arc::new(FakeLauncher::default()),
        jobs: Arc::new(FakeJobs::default()),
        clock: Arc::new(FixedClock(now)),
    }
}

type Script = dyn Fn(&GraphQlRequest) -> Result<GraphQlResponse, TransportError> + Send + Sync;

/// A GraphQL transport answering from a closure, recording every request.
pub struct ScriptedGraphQl {
    script: Box<Script>,
    requests: Mutex<Vec<GraphQlRequest>>,
}

impl ScriptedGraphQl {
    pub fn new(
        script: impl Fn(&GraphQlRequest) -> Result<GraphQlResponse, TransportError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            script: Box::new(script),
            requests: Mutex::default(),
        }
    }

    pub fn requests(&self) -> Vec<GraphQlRequest> {
        self.requests.lock().expect("lock").clone()
    }
}

impl GraphQl for ScriptedGraphQl {
    fn execute(&self, request: &GraphQlRequest) -> Result<GraphQlResponse, TransportError> {
        self.requests.lock().expect("lock").push(request.clone());
        (self.script)(request)
    }
}

/// The operation name of a request (`query OwnerRepos(…)` gives "OwnerRepos").
pub fn operation(request: &GraphQlRequest) -> String {
    request
        .query
        .lines()
        .find_map(|l| l.trim().strip_prefix("query "))
        .and_then(|rest| rest.split(['(', ' ', '{']).next())
        .unwrap_or_default()
        .to_owned()
}

/// A response with `data` and no errors.
pub fn data(value: serde_json::Value) -> GraphQlResponse {
    GraphQlResponse {
        data: Some(value),
        errors: Vec::new(),
    }
}
