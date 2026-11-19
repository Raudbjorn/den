//! What every fetcher shares: running a query, turning its outcome into a
//! [`SourceState`], walking pages, and remembering when the rate limit resets.

use super::graphql::{GraphQl, GraphQlRequest, GraphQlResponse, TransportError};
use crate::{
    domain::{CoverageEntry, Incompleteness, Source, SourceState},
    ports::SurveyFailure,
};
use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Mutex;

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

/// The latest rate-limit reset time GitHub reported in any response of a
/// survey, so a later RATE_LIMITED error (which carries none) can say when the
/// limit resets. Shared by the survey's parallel Owner sessions.
#[derive(Debug, Default)]
pub struct RateLimitMemo(Mutex<Option<Timestamp>>);

impl RateLimitMemo {
    fn get(&self) -> Option<Timestamp> {
        // A poisoned lock only means another thread panicked mid-write of a Copy value.
        self.0.lock().map_or(None, |guard| *guard)
    }

    fn set(&self, at: Timestamp) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = Some(at);
        }
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

    /// Runs `query` and decodes its `data` as `T`. `gh` missing or logged out
    /// stops the whole survey (`Err`); anything else becomes a source state.
    pub fn run<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<Fetched<T>, SurveyFailure> {
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
            Err(TransportError::RateLimited { .. }) => Ok(Fetched::Lost(SourceState::Partial(
                Incompleteness::RateLimited {
                    resets_at: self.rate_limit_reset.get(),
                },
            ))),
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
        let messages: Vec<String> = response.errors.iter().map(|e| e.message.clone()).collect();
        let error_state = if rate_limited {
            SourceState::Partial(Incompleteness::RateLimited {
                resets_at: self.rate_limit_reset.get(),
            })
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
