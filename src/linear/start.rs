//! What Start needs from Linear (#33): each team's workflow states, checked
//! against the team and state names the Linear Workflow configures, and the
//! start teams' unassigned Linear Issues that are not started yet.
//!
//! - **Team states** (`Linear(TeamStates)`): every team the key can read, with
//!   its states. A start team, `in_progress_state` or `release_state` that
//!   Linear doesn't have makes the source **failed**, naming each bad value
//!   (spec 0002, "Config"), so a typo never quietly turns a rule off. A name
//!   den couldn't check (a team list or a team's states den couldn't read in
//!   full) is never called missing: the source is partial instead.
//! - **Start-team issues** (`Linear(StartTeamIssues)`): unassigned issues of
//!   the start teams whose state type is unstarted (and backlog when the
//!   workflow includes it), limited to the active cycle when it asks. Linear
//!   filters; the decision engine still applies the same rule to whatever it
//!   is given.

use super::{
    Connection, LINEAR_PAGE_SIZE, RawIssue, RawState, issue_page_variables,
    session::{Answer, Pager, Session},
};
use crate::{
    config::LinearWorkflow,
    domain::{
        Incompleteness, LinearTeamStates, SourceState,
        facts::{LinearIssue, LinearState, LinearStateKind, LinearTeam},
    },
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashSet;

const TEAM_STATES_QUERY: &str = include_str!("queries/team_states.graphql");
/// Spreads #35's `LinearIssueFields`, so a Start offer carries its
/// attachments like any other Linear Issue.
const START_TEAM_ISSUES_QUERY: &str = with_issue_fields!("queries/start_team_issues.graphql");

/// Workflow states read per team. A team with more is read partially, and
/// its state names aren't checked.
pub const MAX_TEAM_STATES: u32 = 50;

/// The teams' workflow states, and the source's state: failed when the
/// config names a team or state Linear doesn't have.
pub(super) fn team_states(
    session: &Session<'_>,
    workflow: &LinearWorkflow,
) -> (Vec<LinearTeamStates>, SourceState) {
    let mut teams: Vec<LinearTeamStates> = Vec::new();
    // Teams whose states den couldn't read in full, by team ID.
    let mut truncated: HashSet<String> = HashSet::new();
    let state = read_pages::<TeamsData>(
        session,
        "LinearTeamStates",
        TEAM_STATES_QUERY,
        |after| json!({ "first": LINEAR_PAGE_SIZE, "after": after, "statesFirst": MAX_TEAM_STATES }),
        |data| data.teams,
        "Linear team(s)",
        |node, _page_complete| {
            let raw: RawTeamStates = serde_json::from_value(node).map_err(|e| e.to_string())?;
            if teams.iter().any(|t| t.team.id == raw.id) {
                return Ok(());
            }
            if raw.states.page_info.has_next_page {
                truncated.insert(raw.id.clone());
            }
            teams.push(raw.into_team_states(workflow));
            Ok(())
        },
    );
    // An unreadable team node could be a start team, so a name is called
    // missing only when every team was read.
    let teams_read_in_full = state == SourceState::Complete;
    let mut truncated_keys: Vec<&str> = teams
        .iter()
        .filter(|t| truncated.contains(&t.team.id))
        .map(|t| t.team.key.as_str())
        .collect();
    truncated_keys.sort_unstable();
    let read_state = match truncated_keys.first() {
        None => state,
        Some(first) => state.worst(SourceState::Partial(Incompleteness::FollowUpTruncated {
            what: format!(
                "the workflow states of {} team(s) beyond the first {MAX_TEAM_STATES}, first: {first}",
                truncated_keys.len()
            ),
        })),
    };
    let mismatches = mismatches(workflow, &teams, &truncated, teams_read_in_full);
    let state = if mismatches.is_empty() {
        read_state
    } else {
        SourceState::Failed {
            error: format!(
                "[linear.{}] names what Linear doesn't have: {}",
                workflow.owner,
                mismatches.join("; ")
            ),
        }
    };
    (teams, state)
}

/// Each config value Linear doesn't have, said plainly. A start team is
/// missing only when den read the whole team list; a state name only when it
/// read that team's states in full.
fn mismatches(
    workflow: &LinearWorkflow,
    teams: &[LinearTeamStates],
    truncated: &HashSet<String>,
    teams_read_in_full: bool,
) -> Vec<String> {
    let mut found = Vec::new();
    let mut mismatches = Vec::new();
    for key in &workflow.start_teams {
        match teams.iter().find(|t| t.team.key.eq_ignore_ascii_case(key)) {
            Some(team) => found.push(team),
            None if teams_read_in_full => mismatches.push(format!(
                "start_teams names {key:?}, but the Linear workspace {} has no team with that key",
                workflow.workspace
            )),
            None => {}
        }
    }
    for (setting, name) in workflow.state_names() {
        if workflow.start_teams.is_empty() {
            // No start team to hold it to: it must be some team's state.
            let anywhere = teams.iter().any(|t| has_state(&t.states, name));
            if !anywhere && teams_read_in_full && truncated.is_empty() {
                mismatches.push(format!(
                    "{setting} {name:?} is not a state of any team in the Linear workspace {}",
                    workflow.workspace
                ));
            }
            continue;
        }
        for team in &found {
            if truncated.contains(&team.team.id) || has_state(&team.states, name) {
                continue;
            }
            let hint = team
                .states
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
                .map(|s| format!(" (Linear spells it {:?})", s.name))
                .unwrap_or_default();
            mismatches.push(format!(
                "{setting} {name:?} is not a state of start team {}{hint}",
                team.team.key
            ));
        }
    }
    mismatches
}

/// State names compare exactly, as a state move (#38) will look them up.
fn has_state(states: &[LinearState], name: &str) -> bool {
    states.iter().any(|s| s.name == name)
}

/// The start teams' unassigned, not-yet-started Linear Issues (see the
/// module docs), every page up to the page limit, without those already in
/// `known`.
pub(super) fn start_team_issues(
    session: &Session<'_>,
    workflow: &LinearWorkflow,
    known: &[LinearIssue],
) -> (Vec<LinearIssue>, SourceState) {
    let filter = start_filter(workflow);
    let mut issues: Vec<LinearIssue> = Vec::new();
    let mut seen: HashSet<String> = known
        .iter()
        .filter(|i| i.id.workspace == workflow.workspace)
        .map(|i| i.id.id.clone())
        .collect();
    let state = read_pages::<IssuesData>(
        session,
        "LinearStartTeamIssues",
        START_TEAM_ISSUES_QUERY,
        |after| {
            let mut variables = issue_page_variables(after);
            variables["filter"] = filter.clone();
            variables
        },
        |data| data.issues,
        "Linear Issue(s)",
        |node, page_complete| {
            let raw: RawIssue = serde_json::from_value(node).map_err(|e| e.to_string())?;
            if seen.insert(raw.id.clone()) {
                issues.push(raw.into_issue(workflow, page_complete));
            }
            Ok(())
        },
    );
    (issues, state)
}

/// Linear's `IssueFilter` for Start offers. Team keys match case-insensitively,
/// as the team-states check and the decision engine compare them.
fn start_filter(workflow: &LinearWorkflow) -> Value {
    let teams: Vec<Value> = workflow
        .start_teams
        .iter()
        .map(|key| json!({ "key": { "eqIgnoreCase": key } }))
        .collect();
    let mut types = vec![LinearStateKind::Unstarted.label()];
    if workflow.include_backlog {
        types.push(LinearStateKind::Backlog.label());
    }
    let mut filter = json!({
        "team": { "or": teams },
        "assignee": { "null": true },
        "state": { "type": { "in": types } },
    });
    if workflow.active_cycle_only {
        filter["cycle"] = json!({ "isActive": { "eq": true } });
    }
    filter
}

/// Reads every page of one connection, up to the page limit, handing each
/// node to `each` with whether its page came without errors (#35: errors
/// next to data leave that page's attachments incomplete); a node `each`
/// can't read makes the source partial (`what` names the nodes in that
/// reason).
fn read_pages<T: DeserializeOwned>(
    session: &Session<'_>,
    operation: &str,
    query: &str,
    variables: impl Fn(Option<&str>) -> Value,
    connection: impl Fn(T) -> Connection,
    what: &str,
    mut each: impl FnMut(Value, bool) -> Result<(), String>,
) -> SourceState {
    let mut pager = Pager::default();
    let mut unreadable: Vec<String> = Vec::new();
    loop {
        let (page, page_complete) =
            match session.run::<T>(operation, query, variables(pager.cursor.as_deref())) {
                Answer::Lost(state) => {
                    pager.page_lost(state);
                    break;
                }
                Answer::Data { data, state } => {
                    let complete = state.is_complete();
                    pager.page_read(state);
                    (connection(data), complete)
                }
            };
        for node in page.nodes {
            if let Err(e) = each(node, page_complete) {
                unreadable.push(e);
            }
        }
        if !page.page_info.has_next_page {
            break;
        }
        let Some(cursor) = page.page_info.end_cursor else {
            pager.note(SourceState::Partial(Incompleteness::PaginationStopped {
                pages_read: pager.pages_read(),
                reason: "Linear said there were more pages but gave no cursor".to_owned(),
            }));
            break;
        };
        pager.cursor = Some(cursor);
        if !pager.may_continue() {
            break;
        }
    }
    if let Some(first) = unreadable.first() {
        pager.note(SourceState::Partial(Incompleteness::FollowUpTruncated {
            what: format!(
                "{} {what} den could not read, first: {first}",
                unreadable.len()
            ),
        }));
    }
    pager.finish()
}

#[derive(Deserialize)]
struct TeamsData {
    teams: Connection,
}

#[derive(Deserialize)]
struct IssuesData {
    issues: Connection,
}

#[derive(Deserialize)]
struct RawTeamStates {
    id: String,
    key: String,
    name: String,
    states: RawStates,
}

#[derive(Deserialize)]
struct RawStates {
    nodes: Vec<RawState>,
    #[serde(rename = "pageInfo")]
    page_info: StatesPageInfo,
}

#[derive(Deserialize)]
struct StatesPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
}

impl RawTeamStates {
    fn into_team_states(self, workflow: &LinearWorkflow) -> LinearTeamStates {
        LinearTeamStates {
            owner: workflow.owner.clone(),
            workspace: workflow.workspace.clone(),
            team: LinearTeam {
                id: self.id,
                key: self.key,
            },
            name: self.name,
            states: self
                .states
                .nodes
                .into_iter()
                .map(RawState::into_state)
                .collect(),
        }
    }
}
