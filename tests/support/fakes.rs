//! Fake ports for seam-2 tests, and a scripted GraphQL transport for the GitHub
//! adapter tests. Later tickets add methods to `FakeWorkspace` / `FakeLauncher`
//! as the port traits grow.

use den::{
    cache::{CacheError, CachedObservation},
    domain::Observation,
    github::graphql::{GraphQl, GraphQlRequest, GraphQlResponse, TransportError},
    ports::{
        Clock, GitHubSource, Launcher, ObservationCache, Ports, SurveyFailure, SurveyRequest,
        Workspace,
    },
};
use jiff::Timestamp;
use std::{
    collections::VecDeque,
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

#[derive(Default)]
pub struct FakeWorkspace;
impl Workspace for FakeWorkspace {}

#[derive(Default)]
pub struct FakeLauncher;
impl Launcher for FakeLauncher {}

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
        workspace: Arc::new(FakeWorkspace),
        launcher: Arc::new(FakeLauncher),
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
