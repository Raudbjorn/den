//! GraphQL over `gh api graphql`: the transport behind every fetcher.
//!
//! `gh` exits non-zero when a response carries errors, but still prints the body.
//! So the body is parsed first and the exit code only matters when there is no
//! GraphQL body: then stderr says whether `gh` is logged out, rate limited, or
//! something else failed.

use crate::process::{self, ProcessError};
use serde::Deserialize;
use serde_json::Value;
use std::{
    ffi::OsString,
    time::{Duration, Instant},
};

/// Timeout for one `gh` call, in seconds.
pub const GH_TIMEOUT_SECS: u64 = 30;
/// The most of `gh`'s stderr quoted in an error, in characters.
const MAX_STDERR_CHARS: usize = 400;
/// `gh`'s exit code when it has no credentials.
const GH_EXIT_AUTH_REQUIRED: i32 = 4;

/// One GraphQL request: the query text and its variables.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphQlRequest {
    pub query: String,
    pub variables: Value,
}

/// A GraphQL response. `data` and `errors` may both be present: that is a partial
/// result, never a success.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct GraphQlResponse {
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub errors: Vec<GraphQlError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GraphQlError {
    pub message: String,
    /// GitHub's error type, e.g. `NOT_FOUND`, `FORBIDDEN`, `RATE_LIMITED`.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

impl GraphQlError {
    pub fn is_rate_limit(&self) -> bool {
        self.kind.as_deref() == Some("RATE_LIMITED")
            || self.message.to_ascii_lowercase().contains("rate limit")
    }
}

/// Why a request produced no GraphQL body at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("the GitHub CLI (`gh`) is not installed or not on PATH")]
    GhNotInstalled,
    #[error("`gh` is not logged in to GitHub: {detail}")]
    NotLoggedIn { detail: String },
    #[error("GitHub's rate limit was hit: {detail}")]
    RateLimited { detail: String },
    #[error("`gh` did not answer within {after:?}")]
    TimedOut { after: Duration },
    #[error("{detail}")]
    Failed { detail: String },
}

/// Executes GraphQL requests. The real one is [`GhCli`]; tests use canned responses.
pub trait GraphQl: Send + Sync {
    fn execute(&self, request: &GraphQlRequest) -> Result<GraphQlResponse, TransportError>;
}

/// `gh api graphql --input -`, with the request body on stdin.
#[derive(Debug, Clone)]
pub struct GhCli {
    program: OsString,
    timeout: Duration,
}

impl GhCli {
    /// `gh` from PATH, with [`GH_TIMEOUT_SECS`].
    pub fn new() -> Self {
        Self::with_program("gh", Duration::from_secs(GH_TIMEOUT_SECS))
    }

    /// A specific `gh` executable and timeout (tests point this at a fake script).
    pub fn with_program(program: impl Into<OsString>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            timeout,
        }
    }
}

impl Default for GhCli {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQl for GhCli {
    fn execute(&self, request: &GraphQlRequest) -> Result<GraphQlResponse, TransportError> {
        let body = serde_json::json!({ "query": request.query, "variables": request.variables });
        let input = serde_json::to_vec(&body).map_err(|e| TransportError::Failed {
            detail: format!("could not encode the GraphQL request: {e}"),
        })?;
        let started = Instant::now();
        let output = process::run(
            &self.program,
            &["api", "graphql", "--input", "-"],
            &input,
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
            operation = operation_name(&request.query),
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            exit = output.status.code(),
            ok = result.is_ok(),
            "gh api graphql"
        );
        result
    }
}

/// Turns `gh`'s exit code, stdout and stderr into a response or a transport error.
fn classify(
    exit_code: Option<i32>,
    stdout: &[u8],
    stderr: &str,
) -> Result<GraphQlResponse, TransportError> {
    let stderr = stderr.trim();
    let body: Option<Value> = serde_json::from_slice(stdout).ok();
    if let Some(Value::Object(fields)) = &body {
        if fields.contains_key("data") || fields.contains_key("errors") {
            return serde_json::from_value(Value::Object(fields.clone())).map_err(|e| {
                TransportError::Failed {
                    detail: format!("GitHub's GraphQL response had an unexpected shape: {e}"),
                }
            });
        }
        // A REST-style error body, e.g. {"message":"Bad credentials","status":"401"}.
        let message = fields
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let status = fields
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status == "401" {
            return Err(TransportError::NotLoggedIn {
                detail: format!("GitHub rejected gh's token ({message})"),
            });
        }
    }
    if exit_code == Some(0) {
        return Err(TransportError::Failed {
            detail: "`gh` exited successfully but printed no GraphQL response".to_owned(),
        });
    }
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
            "`gh` failed (exit {}): {}",
            exit_code.map_or_else(|| "by signal".to_owned(), |c| c.to_string()),
            quote(stderr)
        ),
    })
}

/// The operation name of a query (`query Viewer {` gives `Viewer`), for logs.
fn operation_name(query: &str) -> &str {
    query
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("query "))
        .and_then(|rest| rest.split(['(', ' ', '{']).next())
        .unwrap_or("anonymous")
}

fn quote(stderr: &str) -> String {
    let first_lines: String = stderr.chars().take(MAX_STDERR_CHARS).collect();
    if first_lines.is_empty() {
        "no error output".to_owned()
    } else {
        first_lines.replace('\n', " ")
    }
}
