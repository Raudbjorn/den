//! HTTPS to Linear's GraphQL endpoint: the transport behind the Linear source.
//!
//! The API key lives only inside [`HttpsTransport`] (as an [`ApiKey`], whose
//! `Debug` hides it) and in the `Authorization` header it sends. A
//! [`LinearRequest`] never carries it, nothing here logs headers, and the
//! reply keeps only the status, the rate-limit headers and the body. So the
//! key can't reach den's log, cache, coverage or Job records through this
//! module.

use serde_json::Value;
use std::{fmt, time::Duration};

/// Linear's GraphQL endpoint.
pub const LINEAR_API_URL: &str = "https://api.linear.app/graphql";
/// Timeout for one Linear request (connect, send and read), in seconds.
pub const LINEAR_TIMEOUT_SECS: u64 = 20;
/// The largest response body den reads from Linear, in bytes.
pub const MAX_LINEAR_BODY_BYTES: u64 = 8 * 1024 * 1024;
/// The response headers den keeps: Linear's rate-limit headers and
/// `Retry-After`. Everything else is dropped unread.
const KEPT_HEADER_PREFIXES: [&str; 2] = ["x-ratelimit-", "retry-after"];

/// A Linear API key, read from the environment variable a Linear Workflow
/// names. Its `Debug` never shows it and it has no `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key itself, for the `Authorization` header and nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey([redacted])")
    }
}

/// One GraphQL request, without credentials.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearRequest {
    /// The operation name, for logs.
    pub operation: String,
    pub query: String,
    pub variables: Value,
}

/// What came back over HTTP: the status, the rate-limit headers (lowercased
/// names) and the body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpReply {
    /// A kept header's value, by lowercased name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Why there was no HTTP reply at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("Linear did not answer within {after:?}")]
    TimedOut { after: Duration },
    #[error("could not reach Linear: {detail}")]
    Network { detail: String },
}

/// Sends GraphQL requests to Linear. The real one is [`HttpsTransport`];
/// tests script replies.
pub trait LinearTransport: Send + Sync {
    fn post(&self, request: &LinearRequest) -> Result<HttpReply, TransportError>;
}

/// The real transport: one blocking HTTPS POST per request, with
/// [`LINEAR_TIMEOUT_SECS`] for the whole call, no redirects (so the key is
/// only ever sent to [`LINEAR_API_URL`]) and HTTPS only.
pub struct HttpsTransport {
    agent: ureq::Agent,
    key: ApiKey,
    timeout: Duration,
}

impl fmt::Debug for HttpsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpsTransport")
            .field("key", &self.key)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl HttpsTransport {
    pub fn new(key: ApiKey) -> Self {
        let timeout = Duration::from_secs(LINEAR_TIMEOUT_SECS);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .https_only(true)
            .max_redirects(0)
            .user_agent(concat!("den/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            agent,
            key,
            timeout,
        }
    }
}

impl LinearTransport for HttpsTransport {
    fn post(&self, request: &LinearRequest) -> Result<HttpReply, TransportError> {
        let body = serde_json::json!({ "query": request.query, "variables": request.variables });
        let body = serde_json::to_vec(&body).map_err(|e| TransportError::Network {
            detail: format!("could not encode the request: {e}"),
        })?;
        let response = self
            .agent
            .post(LINEAR_API_URL)
            .header("Authorization", self.key.expose())
            .header("Content-Type", "application/json")
            .send(&body[..])
            .map_err(|e| self.transport_error(&e))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                let name = name.as_str().to_ascii_lowercase();
                let kept = KEPT_HEADER_PREFIXES.iter().any(|p| name.starts_with(p));
                kept.then(|| Some((name, value.to_str().ok()?.to_owned())))
                    .flatten()
            })
            .collect();
        let body = response
            .into_body()
            .into_with_config()
            .limit(MAX_LINEAR_BODY_BYTES)
            .read_to_vec()
            .map_err(|e| self.transport_error(&e))?;
        Ok(HttpReply {
            status,
            headers,
            body,
        })
    }
}

impl HttpsTransport {
    fn transport_error(&self, error: &ureq::Error) -> TransportError {
        match error {
            ureq::Error::Timeout(_) => TransportError::TimedOut {
                after: self.timeout,
            },
            other => TransportError::Network {
                detail: other.to_string(),
            },
        }
    }
}
