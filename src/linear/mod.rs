//! The real [`LinearSource`](crate::ports::LinearSource): Linear's GraphQL API
//! over HTTPS (spec 0002, "Linear source"). Read-only: every query here is a
//! `query`, never a `mutation`.
//!
//! For each Linear Workflow, in config order: the API key is read from the
//! environment variable the workflow names, then the Linear viewer (checked
//! against the configured workspace), then the viewer's active assigned
//! Linear Issues, paged up to [`MAX_LINEAR_PAGES`], then, when the workflow
//! names start teams or states, the team states and the start teams'
//! unassigned issues ([`start`], #33). Each is recorded as
//! `Source::Owner { owner, kind: SourceKind::Linear(..) }`. Owners without a
//! Linear Workflow are never in the request, so they make no Linear request
//! and need no key.
//!
//! - [`transport`]: the HTTPS transport (timeout per call, the key only in
//!   its header) and the scripted-transport seam.
//! - [`session`]: replies to source states, paging, the rate-limit stop.

mod pages;
pub mod session;
mod start;
pub mod transport;

pub use session::{LINEAR_PAGE_SIZE, MAX_LINEAR_PAGES};
pub use start::MAX_TEAM_STATES;
pub use transport::{
    ApiKey, HttpReply, HttpsTransport, LINEAR_TIMEOUT_SECS, LinearRequest, LinearTransport,
    TransportError,
};

use crate::{
    config::LinearWorkflow,
    domain::{
        Coverage, LinearIssueId, LinearSourceKind, LinearViewer, Source, SourceKind, SourceState,
        facts::{
            LinearCycle, LinearIssue, LinearPriority, LinearState, LinearStateKind, LinearTeam,
            LinearUser,
        },
    },
    ports::{Clock, LinearSource, LinearSurvey, LinearSurveyRequest},
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};
use session::{Answer, Pager, Session};
use std::{collections::HashSet, ffi::OsString, sync::Arc};

const VIEWER_QUERY: &str = include_str!("queries/viewer.graphql");
const ASSIGNED_ISSUES_QUERY: &str = include_str!("queries/assigned_issues.graphql");

/// Reads an environment variable (the real one is `std::env::var_os`).
pub type Env = dyn Fn(&str) -> Option<OsString> + Send + Sync;
/// Makes a transport that sends `key` (the real one is [`HttpsTransport::new`]).
pub type Connect = dyn Fn(ApiKey) -> Box<dyn LinearTransport> + Send + Sync;

/// The Linear source.
pub struct LinearApi {
    env: Box<Env>,
    connect: Box<Connect>,
    clock: Arc<dyn Clock>,
}

impl LinearApi {
    /// Keys from the process environment, HTTPS to Linear.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with(
            Box::new(|name| std::env::var_os(name)),
            Box::new(|key| Box::new(HttpsTransport::new(key))),
            clock,
        )
    }

    /// Keys from `env`, transports from `connect` (tests script both).
    pub fn with(env: Box<Env>, connect: Box<Connect>, clock: Arc<dyn Clock>) -> Self {
        Self {
            env,
            connect,
            clock,
        }
    }
}

impl LinearSource for LinearApi {
    fn survey(&self, request: &LinearSurveyRequest) -> LinearSurvey {
        let mut survey = LinearSurvey::default();
        for workflow in &request.workflows {
            self.survey_workflow(workflow, &mut survey);
        }
        survey
    }
}

fn record(coverage: &mut Coverage, owner: &str, kind: LinearSourceKind, state: SourceState) {
    coverage.record(
        Source::Owner {
            owner: owner.to_owned(),
            kind: SourceKind::Linear(kind),
        },
        state,
    );
}

impl LinearApi {
    fn survey_workflow(&self, workflow: &LinearWorkflow, survey: &mut LinearSurvey) {
        let owner = &workflow.owner;
        let sources = workflow.linear_sources();
        // One trouble that stops every source (no key, no viewer) is
        // recorded for each, so none reads as complete.
        let record_all = |coverage: &mut Coverage, state: SourceState| {
            for kind in &sources {
                record(coverage, owner, *kind, state.clone());
            }
        };
        let key = match self.key(workflow) {
            Ok(key) => key,
            Err(error) => {
                record_all(&mut survey.coverage, SourceState::Failed { error });
                return;
            }
        };
        let transport = (self.connect)(key);
        let session = Session::new(
            transport.as_ref(),
            &workflow.api_key_env,
            self.clock.as_ref(),
        );
        match viewer(&session, workflow) {
            Ok((linear_viewer, state)) => {
                record(&mut survey.coverage, owner, LinearSourceKind::Viewer, state);
                survey.part.viewers.push(linear_viewer);
            }
            Err(state) => {
                // Without the viewer den doesn't ask for its issues: the
                // same trouble (a refused key, no network) would stop them.
                record_all(&mut survey.coverage, state);
                return;
            }
        }
        let (issues, state) = assigned_issues(&session, workflow);
        record(
            &mut survey.coverage,
            owner,
            LinearSourceKind::AssignedIssues,
            state,
        );
        survey.part.issues.extend(issues);
        if sources.contains(&LinearSourceKind::TeamStates) {
            let (teams, state) = start::team_states(&session, workflow);
            record(
                &mut survey.coverage,
                owner,
                LinearSourceKind::TeamStates,
                state,
            );
            survey.part.team_states.extend(teams);
        }
        if sources.contains(&LinearSourceKind::StartTeamIssues) {
            let (issues, state) = start::start_team_issues(&session, workflow, &survey.part.issues);
            record(
                &mut survey.coverage,
                owner,
                LinearSourceKind::StartTeamIssues,
                state,
            );
            survey.part.issues.extend(issues);
        }
    }

    /// The workflow's key, or why there is none (naming the variable, never
    /// a value).
    fn key(&self, workflow: &LinearWorkflow) -> Result<ApiKey, String> {
        let variable = &workflow.api_key_env;
        match (self.env)(variable) {
            None => Err(format!(
                "the environment variable {variable} is not set, so den has no Linear API key for {}",
                workflow.owner
            )),
            Some(value) if value.is_empty() => Err(format!(
                "the environment variable {variable} is empty, so den has no Linear API key for {}",
                workflow.owner
            )),
            Some(value) => value
                .into_string()
                .map(ApiKey::new)
                .map_err(|_| format!("the environment variable {variable} is not valid UTF-8")),
        }
    }
}

#[derive(Deserialize)]
struct ViewerData {
    viewer: RawUser,
    organization: RawOrganization,
}

#[derive(Deserialize)]
struct RawOrganization {
    #[serde(rename = "urlKey")]
    url_key: String,
}

#[derive(Deserialize)]
struct RawUser {
    id: String,
    name: String,
}

/// The Linear viewer, checked against the configured workspace: a key for
/// another workspace would show someone else's picture as this Owner's.
fn viewer(
    session: &Session<'_>,
    workflow: &LinearWorkflow,
) -> Result<(LinearViewer, SourceState), SourceState> {
    match session.run::<ViewerData>("LinearViewer", VIEWER_QUERY, json!({})) {
        Answer::Lost(state) => Err(state),
        Answer::Data { data, state } => {
            if !data
                .organization
                .url_key
                .eq_ignore_ascii_case(&workflow.workspace)
            {
                return Err(SourceState::Failed {
                    error: format!(
                        "the API key in {} belongs to the Linear workspace {:?}, not {:?} as configured for {}",
                        workflow.api_key_env,
                        data.organization.url_key,
                        workflow.workspace,
                        workflow.owner
                    ),
                });
            }
            Ok((
                LinearViewer {
                    owner: workflow.owner.clone(),
                    workspace: workflow.workspace.clone(),
                    id: data.viewer.id,
                    name: data.viewer.name,
                },
                state,
            ))
        }
    }
}

#[derive(Deserialize)]
struct AssignedData {
    viewer: AssignedViewer,
}

#[derive(Deserialize)]
struct AssignedViewer {
    #[serde(rename = "assignedIssues")]
    assigned_issues: Connection,
}

#[derive(Deserialize)]
struct Connection {
    nodes: Vec<Value>,
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

#[derive(Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

/// The viewer's active (not completed, not canceled) assigned Linear Issues,
/// every page up to [`MAX_LINEAR_PAGES`].
fn assigned_issues(
    session: &Session<'_>,
    workflow: &LinearWorkflow,
) -> (Vec<LinearIssue>, SourceState) {
    let mut pager = Pager::default();
    let mut issues: Vec<LinearIssue> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut unreadable: Vec<String> = Vec::new();
    let list = pages::List {
        operation: "LinearAssignedIssues",
        query: ASSIGNED_ISSUES_QUERY,
        variables: |after: Option<&str>| json!({ "first": LINEAR_PAGE_SIZE, "after": after }),
        connection: |data: AssignedData| data.viewer.assigned_issues,
    };
    let end = pages::read_list(session, &mut pager, MAX_LINEAR_PAGES, &list, |node| {
        match serde_json::from_value::<RawIssue>(node) {
            Ok(raw) if seen.insert(raw.id.clone()) => issues.push(raw.into_issue(workflow)),
            Ok(_) => {}
            Err(e) => unreadable.push(e.to_string()),
        }
    });
    if end == pages::End::Cut {
        let bound = pages::page_bound(&pager, None);
        pager.note(bound);
    }
    pages::note_unreadable(&mut pager, &unreadable, "Linear Issue(s)");
    (issues, pager.finish())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIssue {
    id: String,
    identifier: String,
    title: String,
    url: String,
    priority: f64,
    created_at: Timestamp,
    updated_at: Timestamp,
    branch_name: Option<String>,
    team: RawTeam,
    state: RawState,
    assignee: Option<RawUser>,
    cycle: Option<RawCycle>,
}

#[derive(Deserialize)]
struct RawTeam {
    id: String,
    key: String,
}

#[derive(Deserialize)]
struct RawState {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCycle {
    id: String,
    number: f64,
    name: Option<String>,
    is_active: bool,
}

impl RawState {
    fn into_state(self) -> LinearState {
        LinearState {
            id: self.id,
            name: self.name,
            kind: LinearStateKind::from_linear(&self.kind),
        }
    }
}

impl RawIssue {
    fn into_issue(self, workflow: &LinearWorkflow) -> LinearIssue {
        LinearIssue {
            owner: workflow.owner.clone(),
            id: LinearIssueId::new(workflow.workspace.clone(), self.id),
            identifier: self.identifier,
            title: self.title,
            url: self.url,
            team: LinearTeam {
                id: self.team.id,
                key: self.team.key,
            },
            state: self.state.into_state(),
            assignee: self.assignee.map(|a| LinearUser {
                id: a.id,
                name: a.name,
            }),
            priority: LinearPriority::from_linear(whole(self.priority)),
            cycle: self.cycle.map(|c| LinearCycle {
                id: c.id,
                number: u64::try_from(whole(c.number)).unwrap_or_default(),
                name: c.name,
                active: c.is_active,
            }),
            created_at: self.created_at,
            updated_at: self.updated_at,
            branch_name: self.branch_name.filter(|b| !b.is_empty()),
        }
    }
}

/// Linear sends some integers as GraphQL Floats; a value with a fraction is
/// kept as its whole part.
fn whole(value: f64) -> i64 {
    // `as` saturates on overflow and maps NaN to 0, which is what's wanted here.
    #[allow(clippy::cast_possible_truncation)]
    let whole = value.trunc() as i64;
    whole
}
