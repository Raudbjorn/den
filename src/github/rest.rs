//! REST over `gh api -i <path>`, for the facts GitHub's GraphQL API lacks
//! (workflow Runs: there is no GraphQL listing of a branch's Runs per workflow).
//!
//! `-i` makes `gh` print the HTTP status line and headers before the body, so an
//! HTTP error (which `gh` reports with a non-zero exit) is still a response with
//! its real status, and a rate limit carries its reset time. Only when there is
//! no HTTP response at all does stderr decide what went wrong.

use super::{
    graphql::{GhCli, TransportError},
    session::Fetched,
};
use crate::{
    domain::{Incompleteness, SourceState},
    ports::SurveyFailure,
    process::{self, ProcessError},
};
use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Instant;

/// The most of `gh`'s stderr or an error body quoted in an error, in characters.
const MAX_QUOTED_CHARS: usize = 400;
/// `gh`'s exit code when it has no credentials.
const GH_EXIT_AUTH_REQUIRED: i32 = 4;
const HTTP_OK: u16 = 200;
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_NOT_FOUND: u16 = 404;
const HTTP_TOO_MANY_REQUESTS: u16 = 429;

/// One REST response: the HTTP status, the rate-limit headers, and the JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct RestResponse {
    pub status: u16,
    /// `X-RateLimit-Remaining`, when sent.
    pub rate_limit_remaining: Option<u64>,
    /// `X-RateLimit-Reset`, when sent.
    pub rate_limit_reset: Option<Timestamp>,
    /// The body as JSON; `None` when it was empty or not JSON.
    pub body: Option<Value>,
}

impl RestResponse {
    /// GitHub's `message` field of an error body.
    fn message(&self) -> &str {
        self.body
            .as_ref()
            .and_then(|b| b.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// A primary or secondary rate limit (GitHub answers 403 or 429).
    fn is_rate_limited(&self) -> bool {
        matches!(self.status, HTTP_FORBIDDEN | HTTP_TOO_MANY_REQUESTS)
            && (self.rate_limit_remaining == Some(0)
                || self.message().to_ascii_lowercase().contains("rate limit"))
    }
}

/// Executes REST `GET`s. The real one is [`GhCli`]; tests use canned responses.
pub trait Rest: Send + Sync {
    /// `GET path` (relative to the API root, query string included).
    fn get(&self, path: &str) -> Result<RestResponse, TransportError>;
}

impl Rest for GhCli {
    fn get(&self, path: &str) -> Result<RestResponse, TransportError> {
        let started = Instant::now();
        let output = process::run(
            &self.program,
            &["api", "-i", "--method", "GET", path],
            &[],
            self.timeout,
        )
        .map_err(|error| match error {
            ProcessError::NotFound { .. } => TransportError::GhNotInstalled,
            ProcessError::TimedOut { timeout, .. } => TransportError::TimedOut { after: timeout },
            other => TransportError::Failed {
                detail: other.to_string(),
            },
        })?;
        let result = classify(
            output.status.code(),
            &output.stdout,
            &String::from_utf8_lossy(&output.stderr),
        );
        tracing::info!(
            path = path.split('?').next().unwrap_or(path),
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            exit = output.status.code(),
            http_status = result.as_ref().ok().map(|r| r.status),
            rate_limit_remaining = result.as_ref().ok().and_then(|r| r.rate_limit_remaining),
            "gh api"
        );
        result
    }
}

/// Turns `gh api -i`'s exit code, stdout and stderr into a response or a
/// transport error.
fn classify(
    exit_code: Option<i32>,
    stdout: &[u8],
    stderr: &str,
) -> Result<RestResponse, TransportError> {
    if let Some(response) = parse_http(stdout) {
        return Ok(response);
    }
    let stderr = stderr.trim();
    let lower = stderr.to_ascii_lowercase();
    if exit_code == Some(GH_EXIT_AUTH_REQUIRED)
        || lower.contains("gh auth login")
        || lower.contains("http 401")
    {
        return Err(TransportError::NotLoggedIn {
            detail: quote(stderr),
        });
    }
    if lower.contains("rate limit") {
        return Err(TransportError::RateLimited {
            detail: quote(stderr),
        });
    }
    Err(TransportError::Failed {
        detail: format!(
            "`gh api` printed no HTTP response (exit {}): {}",
            exit_code.map_or_else(|| "by signal".to_owned(), |c| c.to_string()),
            quote(stderr)
        ),
    })
}

/// Parses `HTTP/2.0 200 OK`, headers, a blank line and the body.
fn parse_http(stdout: &[u8]) -> Option<RestResponse> {
    let text = String::from_utf8_lossy(stdout);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .or_else(|| text.split_once("\n\n"))
        .unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status_line = lines.next()?;
    if !status_line.starts_with("HTTP/") {
        return None;
    }
    let status = status_line.split_whitespace().nth(1)?.parse::<u16>().ok()?;
    let mut rate_limit_remaining = None;
    let mut rate_limit_reset = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "x-ratelimit-remaining" => rate_limit_remaining = value.parse().ok(),
            "x-ratelimit-reset" => {
                rate_limit_reset = value
                    .parse::<i64>()
                    .ok()
                    .and_then(|secs| Timestamp::from_second(secs).ok());
            }
            _ => {}
        }
    }
    // `gh` appends its own "gh: Not Found (HTTP 404)" line after an error body;
    // the JSON body is the first value in the stream.
    let body = serde_json::Deserializer::from_str(body)
        .into_iter::<Value>()
        .next()
        .and_then(Result::ok);
    Some(RestResponse {
        status,
        rate_limit_remaining,
        rate_limit_reset,
        body,
    })
}

/// `GET path` and decodes a 200 body as `T`. `gh` missing or logged out stops
/// the whole survey (`Err`); anything else becomes a source state, as for
/// GraphQL ([`super::session::Session::run`]).
pub fn get_json<T: DeserializeOwned>(
    rest: &dyn Rest,
    path: &str,
) -> Result<Fetched<T>, SurveyFailure> {
    let response = match rest.get(path) {
        Ok(response) => response,
        Err(TransportError::GhNotInstalled) => return Err(SurveyFailure::GhNotInstalled),
        Err(TransportError::NotLoggedIn { detail }) => {
            return Err(SurveyFailure::NotLoggedIn { detail });
        }
        Err(TransportError::RateLimited { .. }) => {
            return Ok(Fetched::Lost(rate_limited(None)));
        }
        Err(other) => {
            return Ok(Fetched::Lost(SourceState::Failed {
                error: other.to_string(),
            }));
        }
    };
    if response.is_rate_limited() {
        return Ok(Fetched::Lost(rate_limited(response.rate_limit_reset)));
    }
    match response.status {
        HTTP_OK => {}
        HTTP_UNAUTHORIZED => {
            return Err(SurveyFailure::NotLoggedIn {
                detail: format!("GitHub rejected gh's token ({})", response.message()),
            });
        }
        HTTP_NOT_FOUND => {
            return Ok(Fetched::Lost(SourceState::Failed {
                error: "GitHub answered 404: not found, or gh's token cannot see it".to_owned(),
            }));
        }
        status => {
            return Ok(Fetched::Lost(SourceState::Failed {
                error: format!(
                    "GitHub answered HTTP {status}: {}",
                    quote(response.message())
                ),
            }));
        }
    }
    let Some(body) = response.body else {
        return Ok(Fetched::Lost(SourceState::Failed {
            error: "GitHub answered 200 without a JSON body".to_owned(),
        }));
    };
    Ok(match serde_json::from_value::<T>(body) {
        Ok(data) => Fetched::Data {
            data,
            state: SourceState::Complete,
        },
        Err(e) => Fetched::Lost(SourceState::Failed {
            error: format!("GitHub's response had an unexpected shape: {e}"),
        }),
    })
}

fn rate_limited(resets_at: Option<Timestamp>) -> SourceState {
    SourceState::Partial(Incompleteness::RateLimited { resets_at })
}

/// Percent-encodes one query-string or path value (everything but RFC 3986's
/// unreserved characters), so branch names with `/`, `#`, `&` or `+` survive.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn quote(text: &str) -> String {
    let first: String = text.chars().take(MAX_QUOTED_CHARS).collect();
    if first.is_empty() {
        "no error output".to_owned()
    } else {
        first.replace('\n', " ")
    }
}
