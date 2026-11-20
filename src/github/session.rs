//! What every fetcher shares: running a query, turning its outcome into a
//! [`SourceState`], walking pages, and remembering when the rate limit resets.

use super::{
    graphql::{GraphQl, GraphQlRequest, GraphQlResponse, TransportError},
    rest::Rest,
};
use crate::{
    domain::{CoverageEntry, Incompleteness, Source, SourceState},
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::{Mutex, PoisonError};

/// Upper bound on pages read per connection, so a runaway pagination can't run
/// forever. Reaching it is recorded as partial coverage.
pub const MAX_PAGES: u32 = 50;

/// The result of one query, when the survey can go on.
pub enum Fetched<T> {
    /// A response; `state` is complete, or partial if errors came alongside data.
    Data { data: T, state: SourceState },
    /// Nothing usable came back; `state` says why.
    Lost(SourceState),
}

/// What one kind's fetcher found for one Owner.
#[derive(Debug)]
pub struct KindFetch<T> {
    pub items: Vec<T>,
    pub coverage: Vec<CoverageEntry>,
}

impl<T> Default for KindFetch<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            coverage: Vec::new(),
        }
    }
}

/// A survey's rate-limit state, shared by its parallel Owner sessions: the
/// latest reset time GitHub reported in any response (so a later RATE_LIMITED
/// error, which carries none, can say when the limit resets), and whether the
/// limit was hit. Once it was, the survey stops fetching: every later query is
/// recorded as rate limited without being sent.
#[derive(Debug, Default)]
pub struct RateLimitMemo(Mutex<RateLimitState>);

#[derive(Debug, Default, Clone, Copy)]
struct RateLimitState {
    resets_at: Option<Timestamp>,
    hit: bool,
    /// den already asked GitHub when the limit resets.
    asked: bool,
}

impl RateLimitMemo {
    fn state(&self) -> RateLimitState {
        self.update(|s| *s)
    }

    /// Reads and changes the state under one lock, so a check and the change it
    /// decides on can't interleave with another Owner's thread.
    fn update<R>(&self, change: impl FnOnce(&mut RateLimitState) -> R) -> R {
        // A poisoned lock only means another thread panicked while holding it;
        // every change is a few plain field writes, so the state is still sound.
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut guard)
    }

    fn get(&self) -> Option<Timestamp> {
        self.state().resets_at
    }

    fn set(&self, at: Timestamp) {
        self.update(|s| s.resets_at = Some(at));
    }

    /// Records that the limit was hit. If no response said when it resets, asks
    /// GitHub once (that question does not count against the limit). The one
    /// question is claimed under the lock, so parallel Owner threads hitting the
    /// limit together still ask only once.
    fn hit(&self, gql: &dyn GraphQl) {
        let should_ask = self.update(|s| {
            let should_ask = s.resets_at.is_none() && !s.asked;
            s.hit = true;
            s.asked = true;
            should_ask
        });
        if should_ask && let Some(at) = gql.rate_limit_reset() {
            self.set(at);
        }
    }

    fn limited(&self) -> SourceState {
        SourceState::Partial(Incompleteness::RateLimited {
            resets_at: self.get(),
        })
    }
}

pub struct Session<'a> {
    gql: &'a dyn GraphQl,
    rate_limit_reset: &'a RateLimitMemo,
}

impl<'a> Session<'a> {
    pub fn new(gql: &'a dyn GraphQl, rate_limit_reset: &'a RateLimitMemo) -> Self {
        Self {
            gql,
            rate_limit_reset,
        }
    }

    /// The transport's REST side, when it has one (see [`GraphQl::rest`]).
    pub fn rest(&self) -> Option<&'a dyn Rest> {
        self.gql.rest()
    }

    /// Runs `query` and decodes its `data` as `T`. `gh` missing or logged out
    /// stops the whole survey (`Err`); anything else becomes a source state.
    pub fn run<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<Fetched<T>, SurveyFailure> {
        if self.rate_limit_reset.state().hit {
            return Ok(Fetched::Lost(self.rate_limit_reset.limited()));
        }
        let request = GraphQlRequest {
            query: query.to_owned(),
            variables,
        };
        match self.gql.execute(&request) {
            Ok(response) => Ok(self.decode(response)),
            Err(TransportError::GhNotInstalled) => Err(SurveyFailure::GhNotInstalled),
            Err(TransportError::NotLoggedIn { detail }) => {
                Err(SurveyFailure::NotLoggedIn { detail })
            }
            Err(TransportError::RateLimited { .. }) => {
                self.rate_limit_reset.hit(self.gql);
                Ok(Fetched::Lost(self.rate_limit_reset.limited()))
            }
            Err(other) => Ok(Fetched::Lost(SourceState::Failed {
                error: other.to_string(),
            })),
        }
    }

    fn decode<T: DeserializeOwned>(&self, response: GraphQlResponse) -> Fetched<T> {
        if let Some(reset) = response
            .data
            .as_ref()
            .and_then(|d| d.pointer("/rateLimit/resetAt"))
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<Timestamp>().ok())
        {
            self.rate_limit_reset.set(reset);
        }
        let rate_limited = response.errors.iter().any(|e| e.is_rate_limit());
        if rate_limited {
            self.rate_limit_reset.hit(self.gql);
        }
        let messages: Vec<String> = response.errors.iter().map(|e| e.message.clone()).collect();
        let error_state = if rate_limited {
            self.rate_limit_reset.limited()
        } else {
            SourceState::Partial(Incompleteness::ErrorsAlongsideData {
                errors: messages.clone(),
            })
        };
        let data = match response.data {
            Some(Value::Null) | None => {
                return Fetched::Lost(if rate_limited {
                    error_state
                } else {
                    SourceState::Failed {
                        error: if messages.is_empty() {
                            "GitHub returned neither data nor errors".to_owned()
                        } else {
                            messages.join("; ")
                        },
                    }
                });
            }
            Some(data) => data,
        };
        match serde_json::from_value::<T>(data) {
            Ok(data) => Fetched::Data {
                data,
                state: if response.errors.is_empty() {
                    SourceState::Complete
                } else {
                    error_state
                },
            },
            Err(e) => Fetched::Lost(SourceState::Failed {
                error: format!("GitHub's response had an unexpected shape: {e}"),
            }),
        }
    }
}

/// Tracks one paginated connection: pages read and the worst state so far.
pub struct Pager {
    pub cursor: Option<String>,
    pages_read: u32,
    state: SourceState,
}

impl Pager {
    pub fn new(cursor: Option<String>) -> Self {
        Self {
            cursor,
            pages_read: 0,
            state: SourceState::Complete,
        }
    }

    /// False once [`MAX_PAGES`] pages were read; the bound is recorded as partial.
    pub fn may_continue(&mut self) -> bool {
        if self.pages_read < MAX_PAGES {
            return true;
        }
        self.note(SourceState::Partial(Incompleteness::PaginationStopped {
            pages_read: self.pages_read,
            reason: format!("den reads at most {MAX_PAGES} pages per list"),
        }));
        false
    }

    /// A page arrived with this state.
    pub fn page_read(&mut self, state: SourceState) {
        self.pages_read += 1;
        self.note(state);
    }

    /// A page was lost. After earlier pages, that makes the list partial.
    pub fn page_lost(&mut self, state: SourceState) {
        let state = match (self.pages_read, state) {
            (0, state) => state,
            (_, SourceState::Partial(Incompleteness::RateLimited { resets_at })) => {
                SourceState::Partial(Incompleteness::RateLimited { resets_at })
            }
            (pages_read, state) => SourceState::Partial(Incompleteness::PaginationStopped {
                pages_read,
                reason: state.describe(),
            }),
        };
        self.note(state);
    }

    pub fn note(&mut self, state: SourceState) {
        let previous = std::mem::replace(&mut self.state, SourceState::Complete);
        self.state = previous.worst(state);
    }

    /// Moves to the next page, or returns false when there is none.
    pub fn advance(&mut self, has_next_page: bool, end_cursor: Option<String>) -> bool {
        match (has_next_page, end_cursor) {
            (true, Some(cursor)) => {
                self.cursor = Some(cursor);
                true
            }
            (true, None) => {
                self.note(SourceState::Partial(Incompleteness::PaginationStopped {
                    pages_read: self.pages_read,
                    reason: "GitHub reported more pages but no cursor".to_owned(),
                }));
                false
            }
            (false, _) => false,
        }
    }

    pub fn finish(self, source: Source) -> CoverageEntry {
        CoverageEntry {
            source,
            state: self.state,
        }
    }
}

/// GraphQL `pageInfo`.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub has_next_page: bool,
    #[serde(default)]
    pub end_cursor: Option<String>,
}

/// A GraphQL connection whose nodes may be null (they are, next to errors).
#[derive(Debug, serde::Deserialize)]
#[serde(
    rename_all = "camelCase",
    bound(deserialize = "T: serde::Deserialize<'de>")
)]
pub struct Connection<T> {
    pub page_info: PageInfo,
    #[serde(default)]
    pub nodes: Vec<Option<T>>,
}

/// `{ login }`.
#[derive(Debug, serde::Deserialize)]
pub struct Actor {
    pub login: String,
}
