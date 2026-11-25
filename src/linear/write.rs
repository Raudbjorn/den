//! The real [`LinearWriter`]: den's only remote writes (ADR-0004), as two
//! fixed GraphQL `mutation`s over the same HTTPS transport as the Linear
//! source. Nothing else in den sends a `mutation`.
//!
//! - **Assign to the Linear viewer**: the viewer is read first with the same
//!   key (and checked against the configured workspace), then
//!   `issueUpdate(input: { assigneeId })` names that viewer. The port takes no
//!   user, so no one else can be assigned.
//! - **Move state**: `issueUpdate(input: { stateId })`, a state of the issue's
//!   team, by ID.
//!
//! Outcomes: **done** when Linear says it made the change and returns the
//! issue (its assignee and state, as Linear returned them); **failed** when
//! Linear answered without making it, or den never sent it; **unknown** when
//! the request may have reached Linear but its answer was lost: a timeout, a
//! network error, a server error, or a reply den can't read. den never
//! retries: the App re-reads the issue instead.

use super::{
    Connect, Env, RawState, RawUser, read_issue_now,
    session::Session,
    transport::{LinearRequest, LinearTransport},
    viewer,
};
use crate::{
    domain::facts::LinearState,
    ports::{Clock, LinearIssueRequest, LinearWriteOutcome, LinearWriter},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const ASSIGN_TO_VIEWER_MUTATION: &str = include_str!("queries/assign_to_viewer.graphql");
const MOVE_STATE_MUTATION: &str = include_str!("queries/move_state.graphql");
/// HTTP statuses from which Linear's server may or may not have made the change.
const HTTP_SERVER_ERRORS: std::ops::Range<u16> = 500..600;
/// The most of one error message den keeps, in characters.
const MAX_ERROR_CHARS: usize = 300;

/// The two Linear writes, through the transport `connect` makes for each
/// workflow's key.
pub struct LinearWrites {
    credentials: Option<Box<super::CredentialResolver>>,
    env: Box<Env>,
    connect: Box<Connect>,
    clock: Arc<dyn Clock>,
}

impl LinearWrites {
    /// Keys from the process environment, HTTPS to Linear.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with(
            Box::new(|name| std::env::var_os(name)),
            Box::new(|key| Box::new(super::HttpsTransport::new(key))),
            clock,
        )
    }

    /// Keys from `env`, transports from `connect` (tests script both).
    pub fn with(env: Box<Env>, connect: Box<Connect>, clock: Arc<dyn Clock>) -> Self {
        Self {
            credentials: None,
            env,
            connect,
            clock,
        }
    }

    pub fn with_credentials(mut self, resolver: Box<super::CredentialResolver>) -> Self {
        self.credentials = Some(resolver);
        self
    }

    fn transport(
        &self,
        request: &LinearIssueRequest,
    ) -> Result<Box<dyn LinearTransport>, super::CredentialError> {
        let key = super::resolve_credential(
            self.credentials.as_deref(),
            self.env.as_ref(),
            &request.workflow,
        )?;
        Ok((self.connect)(key))
    }
}

impl LinearWriter for LinearWrites {
    fn assign_to_viewer(&self, request: &LinearIssueRequest) -> LinearWriteOutcome {
        let transport = match self.transport(request) {
            Ok(transport) => transport,
            Err(error) => {
                return if error.authentication_failed {
                    LinearWriteOutcome::Unauthenticated {
                        error: error.message,
                    }
                } else {
                    not_sent(error.message)
                };
            }
        };
        let session = Session::new(
            transport.as_ref(),
            &request.workflow.api_key_env,
            self.clock.as_ref(),
        );
        if let Some(outcome) = validate_binding(&session, request) {
            return outcome;
        }
        // A read: whatever goes wrong here, nothing was written.
        let me = match viewer(&session, &request.workflow) {
            Ok((me, _)) => me,
            Err(state) => {
                if session.authentication_failed() {
                    return LinearWriteOutcome::Unauthenticated {
                        error: state.describe(),
                    };
                }
                return not_sent(format!(
                    "den couldn't read whose key this is, so it assigned nothing: {}",
                    state.describe()
                ));
            }
        };
        mutate(
            transport.as_ref(),
            "LinearAssignToViewer",
            ASSIGN_TO_VIEWER_MUTATION,
            json!({ "id": request.issue.id, "assigneeId": me.id }),
        )
    }

    fn move_state(&self, request: &LinearIssueRequest, state: &LinearState) -> LinearWriteOutcome {
        let transport = match self.transport(request) {
            Ok(transport) => transport,
            Err(error) => {
                return if error.authentication_failed {
                    LinearWriteOutcome::Unauthenticated {
                        error: error.message,
                    }
                } else {
                    not_sent(error.message)
                };
            }
        };
        let session = Session::new(
            transport.as_ref(),
            &request.workflow.api_key_env,
            self.clock.as_ref(),
        );
        if let Some(outcome) = validate_binding(&session, request) {
            return outcome;
        }
        mutate(
            transport.as_ref(),
            "LinearMoveState",
            MOVE_STATE_MUTATION,
            json!({ "id": request.issue.id, "stateId": state.id }),
        )
    }
}

/// Nothing was sent, so nothing changed.
fn not_sent(error: String) -> LinearWriteOutcome {
    LinearWriteOutcome::Failed { error }
}

/// Sends one write and says how it ended.
fn mutate(
    transport: &dyn LinearTransport,
    operation: &str,
    query: &str,
    variables: Value,
) -> LinearWriteOutcome {
    let request = LinearRequest {
        operation: operation.to_owned(),
        query: query.to_owned(),
        variables,
    };
    let outcome = match transport.post(&request) {
        // It may have reached Linear: den can't tell.
        Err(error) => LinearWriteOutcome::Unknown {
            why: error.to_string(),
        },
        Ok(reply) => decode(reply.status, &reply.body),
    };
    let label = match &outcome {
        LinearWriteOutcome::Done { .. } => "done",
        LinearWriteOutcome::Failed { .. } | LinearWriteOutcome::Unauthenticated { .. } => "failed",
        LinearWriteOutcome::Unknown { .. } => "unknown",
    };
    tracing::info!(operation, outcome = label, "linear write");
    outcome
}

#[derive(Deserialize)]
struct WriteBody {
    #[serde(default)]
    data: Option<WriteData>,
    #[serde(default)]
    errors: Vec<WriteError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteData {
    issue_update: Option<IssuePayload>,
}

#[derive(Deserialize)]
struct IssuePayload {
    success: bool,
    issue: Option<WrittenIssue>,
}

#[derive(Deserialize)]
struct WrittenIssue {
    assignee: Option<RawUser>,
    state: RawState,
}

#[derive(Deserialize)]
struct WriteError {
    #[serde(default)]
    message: String,
    #[serde(default)]
    extensions: Option<WriteErrorExtensions>,
}

#[derive(Deserialize)]
struct WriteErrorExtensions {
    code: Option<String>,
}

/// A write's reply, as an outcome.
fn decode(status: u16, body: &[u8]) -> LinearWriteOutcome {
    if matches!(status, 401 | 403) {
        return LinearWriteOutcome::Unauthenticated {
            error: "Linear refused authorization; reconnect this Repo".to_owned(),
        };
    }
    if HTTP_SERVER_ERRORS.contains(&status) {
        return LinearWriteOutcome::Unknown {
            why: format!(
                "Linear answered HTTP {status}, so den can't tell whether it made the change"
            ),
        };
    }
    let Ok(body) = serde_json::from_slice::<WriteBody>(body) else {
        return LinearWriteOutcome::Unknown {
            why: format!("den couldn't read Linear's answer (HTTP {status})"),
        };
    };
    if body.errors.iter().any(|error| {
        error
            .extensions
            .as_ref()
            .and_then(|ext| ext.code.as_deref())
            .is_some_and(|code| super::session::AUTH_ERROR_CODES.contains(&code))
    }) {
        return LinearWriteOutcome::Unauthenticated {
            error: "Linear refused authorization; reconnect this Repo".to_owned(),
        };
    }
    let errors = body
        .errors
        .iter()
        .map(|e| bounded(&e.message))
        .collect::<Vec<_>>()
        .join("; ");
    match body.data.and_then(|d| d.issue_update) {
        Some(IssuePayload {
            success: true,
            issue: Some(issue),
        }) => LinearWriteOutcome::Done {
            assignee: issue.assignee.map(RawUser::into_user),
            state: issue.state.into_state(),
        },
        // Linear says it made the change but didn't say what the issue is now.
        Some(IssuePayload {
            success: true,
            issue: None,
        }) => LinearWriteOutcome::Unknown {
            why: "Linear said it made the change but returned no issue".to_owned(),
        },
        Some(IssuePayload { success: false, .. }) => LinearWriteOutcome::Failed {
            error: if errors.is_empty() {
                "Linear answered that it didn't make the change".to_owned()
            } else {
                errors
            },
        },
        None if !errors.is_empty() => LinearWriteOutcome::Failed { error: errors },
        None => LinearWriteOutcome::Failed {
            error: format!("Linear answered HTTP {status} without the change"),
        },
    }
}

/// An error message cut to [`MAX_ERROR_CHARS`], on one line.
fn bounded(message: &str) -> String {
    message
        .chars()
        .take(MAX_ERROR_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Rechecks the immutable identity and issue project immediately before a write.
fn validate_binding(
    session: &Session<'_>,
    request: &LinearIssueRequest,
) -> Option<LinearWriteOutcome> {
    request.workflow.binding.as_ref()?;
    match read_issue_now(session, request) {
        crate::ports::LinearIssueRead::Read(_) => None,
        crate::ports::LinearIssueRead::Unauthenticated { error } => {
            Some(LinearWriteOutcome::Unauthenticated { error })
        }
        crate::ports::LinearIssueRead::Failed { error } => Some(not_sent(error)),
    }
}
