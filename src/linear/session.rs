//! One Linear Workflow's conversation with Linear: running a query, turning
//! its reply into a [`SourceState`], walking pages, and stopping once the rate
//! limit is hit (spec 0002, "Linear source": errors next to data are partial,
//! a rate limit is partial with its reset time, auth or network failure is
//! failed; no invented counts).

use super::transport::{HttpReply, LinearRequest, LinearTransport};
use crate::{
    domain::{Incompleteness, SourceState, coverage::MAX_KEPT_LINEAR_ERRORS},
    paging::{self, PagedService},
    ports::Clock,
};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use std::{cell::Cell, time::Instant};

/// Upper bound on pages read per Linear list. Reaching it with more pages left
/// is recorded as partial coverage.
pub const MAX_LINEAR_PAGES: u32 = 20;
/// Linear Issues asked for per page.
pub const LINEAR_PAGE_SIZE: u32 = 50;
/// The most of one error message den keeps, in characters.
const MAX_ERROR_CHARS: usize = 300;
/// Linear's error code for a rate limit.
const RATE_LIMITED_CODE: &str = "RATELIMITED";
/// Linear's error codes for a key it doesn't accept.
const AUTH_ERROR_CODES: [&str; 2] = ["AUTHENTICATION_ERROR", "FORBIDDEN"];
/// HTTP statuses for a key Linear doesn't accept.
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_TOO_MANY_REQUESTS: u16 = 429;
/// Rate-limit reset headers, in UTC epoch milliseconds.
const RESET_MS_HEADERS: [&str; 3] = [
    "x-ratelimit-requests-reset",
    "x-ratelimit-complexity-reset",
    "x-ratelimit-endpoint-requests-reset",
];
/// Seconds to wait before retrying.
const RETRY_AFTER_HEADER: &str = "retry-after";

/// The result of one query.
pub enum Answer<T> {
    /// Data; `state` is complete, or partial if errors came alongside it.
    Data { data: T, state: SourceState },
    /// Nothing usable came back; `state` says why.
    Lost(SourceState),
}

#[derive(Debug, Default, Deserialize)]
struct GraphQlBody {
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    #[serde(default)]
    message: String,
    #[serde(default)]
    extensions: Option<Extensions>,
}

#[derive(Debug, Deserialize)]
struct Extensions {
    #[serde(default)]
    code: Option<String>,
}

impl GraphQlError {
    fn code(&self) -> Option<&str> {
        self.extensions.as_ref()?.code.as_deref()
    }
}

pub struct Session<'a> {
    transport: &'a dyn LinearTransport,
    /// The variable the key came from, to name in errors (never the key).
    key_env: &'a str,
    /// Read when a reply arrives: a `Retry-After` counts from then.
    clock: &'a dyn Clock,
    /// Once Linear rate-limited this workflow: when it resets, if it said.
    /// Every later query is then recorded as rate limited without being sent.
    limited: Cell<Option<Option<Timestamp>>>,
}

impl<'a> Session<'a> {
    pub fn new(transport: &'a dyn LinearTransport, key_env: &'a str, clock: &'a dyn Clock) -> Self {
        Self {
            transport,
            key_env,
            clock,
            limited: Cell::new(None),
        }
    }

    fn rate_limited(&self) -> SourceState {
        SourceState::Partial(Incompleteness::LinearRateLimited {
            resets_at: self.limited.get().flatten(),
        })
    }

    /// Runs one query and decodes its `data` as `T`.
    pub fn run<T: DeserializeOwned>(
        &self,
        operation: &str,
        query: &str,
        variables: Value,
    ) -> Answer<T> {
        if self.limited.get().is_some() {
            return Answer::Lost(self.rate_limited());
        }
        let request = LinearRequest {
            operation: operation.to_owned(),
            query: query.to_owned(),
            variables,
        };
        let started = Instant::now();
        let reply = self.transport.post(&request);
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match reply {
            Err(error) => {
                tracing::warn!(operation, elapsed_ms, %error, "linear graphql");
                Answer::Lost(SourceState::Failed {
                    error: error.to_string(),
                })
            }
            Ok(reply) => {
                let answer = self.decode(&reply);
                tracing::info!(
                    operation,
                    elapsed_ms,
                    status = reply.status,
                    complete = matches!(&answer, Answer::Data { state, .. } if state.is_complete()),
                    "linear graphql"
                );
                answer
            }
        }
    }

    fn decode<T: DeserializeOwned>(&self, reply: &HttpReply) -> Answer<T> {
        let body: Option<GraphQlBody> = serde_json::from_slice(&reply.body).ok();
        let body = body.unwrap_or_default();
        // Only the first few messages are kept (each cut short); the rest are
        // counted, so one large reply can't grow coverage without bound.
        let messages: Vec<String> = body
            .errors
            .iter()
            .take(MAX_KEPT_LINEAR_ERRORS)
            .map(|e| bounded(&e.message))
            .collect();
        let more = body.errors.len().saturating_sub(messages.len());
        let rate_limited = reply.status == HTTP_TOO_MANY_REQUESTS
            || body
                .errors
                .iter()
                .any(|e| e.code() == Some(RATE_LIMITED_CODE));
        if rate_limited {
            self.limited.set(Some(resets_at(reply, self.clock.now())));
        }
        let refused = matches!(reply.status, HTTP_UNAUTHORIZED | HTTP_FORBIDDEN)
            || body
                .errors
                .iter()
                .any(|e| e.code().is_some_and(|c| AUTH_ERROR_CODES.contains(&c)));
        if refused && !rate_limited {
            return Answer::Lost(SourceState::Failed {
                error: format!(
                    "Linear refused the API key in {} (HTTP {}){}",
                    self.key_env,
                    reply.status,
                    first(&messages)
                        .map(|m| format!(": {m}"))
                        .unwrap_or_default()
                ),
            });
        }
        let error_state = if rate_limited {
            self.rate_limited()
        } else {
            SourceState::Partial(Incompleteness::linear_errors(messages.clone(), more))
        };
        let data = match body.data {
            Some(Value::Null) | None => {
                return Answer::Lost(if rate_limited {
                    error_state
                } else {
                    SourceState::Failed {
                        error: match (first(&messages), more) {
                            (None, _) => {
                                format!("Linear answered HTTP {} without data", reply.status)
                            }
                            (Some(_), 0) => messages.join("; "),
                            (Some(_), more) => format!("{}; and {more} more", messages.join("; ")),
                        },
                    }
                });
            }
            Some(data) => data,
        };
        match serde_json::from_value::<T>(data) {
            Ok(data) => Answer::Data {
                data,
                state: if body.errors.is_empty() && !rate_limited {
                    SourceState::Complete
                } else {
                    error_state
                },
            },
            Err(e) => Answer::Lost(SourceState::Failed {
                error: bounded(&format!("Linear's response had an unexpected shape: {e}")),
            }),
        }
    }
}

fn first(messages: &[String]) -> Option<&String> {
    messages.first()
}

/// When the rate limit resets: the latest of the reset headers Linear sent
/// (epoch milliseconds) and `Retry-After` (seconds from now). `None` when
/// Linear said nothing den could read.
fn resets_at(reply: &HttpReply, now: Timestamp) -> Option<Timestamp> {
    let from_headers = RESET_MS_HEADERS
        .iter()
        .filter_map(|name| reply.header(name))
        .filter_map(|v| v.trim().parse::<i64>().ok())
        .filter_map(|ms| Timestamp::from_millisecond(ms).ok());
    let retry_after = reply
        .header(RETRY_AFTER_HEADER)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .and_then(|secs| now.checked_add(SignedDuration::from_secs(secs)).ok());
    from_headers.chain(retry_after).max()
}

/// An error message cut to [`MAX_ERROR_CHARS`], on one line.
fn bounded(message: &str) -> String {
    message
        .chars()
        .take(MAX_ERROR_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Linear, as a paged service: lists of up to [`MAX_LINEAR_PAGES`] pages.
pub struct Linear;

impl PagedService for Linear {
    const MAX_PAGES: u32 = MAX_LINEAR_PAGES;
    const NAME: &'static str = "Linear";
    const LIST: &'static str = "Linear list";
}

/// Tracks one paginated Linear list (the shared [`paging::Pager`]).
pub type Pager = paging::Pager<Linear>;

impl Pager {
    /// The list's state: the worst of its pages.
    pub fn finish(self) -> SourceState {
        self.into_state()
    }
}
