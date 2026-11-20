//! What Start needs from Linear (#33): the start teams' workflow states,
//! checked against the team and state names the Linear Workflow configures,
//! and the start teams' unassigned Linear Issues that are not started yet.
//!
//! - **Team states** (`Linear(TeamStates)`): the start teams, with their
//!   states (every team the key can read when the workflow names no start
//!   team). A start team, `in_progress_state` or `release_state` that Linear
//!   doesn't have makes the source **failed**, naming each bad value (spec
//!   0002, "Config"), so a typo never quietly turns a rule off; anything den
//!   couldn't read on the way is said next to it. A name den couldn't check
//!   (a team list or a team's states den couldn't read in full) is never
//!   called missing: the source is partial instead.
//! - **Start-team issues** (`Linear(StartTeamIssues)`): unassigned issues of
//!   the start teams whose state type is unstarted (and backlog when the
//!   workflow includes it), limited to the active cycle when it asks. Linear
//!   filters; the decision engine still applies the same rule to whatever it
//!   is given. They are read oldest first; when there are more than one page,
//!   most urgent first too, so that den's page bound cuts the end of the
//!   Start order (spec 0002, "Ordering"), never its head.

use super::{
    Connection, LINEAR_PAGE_SIZE, MAX_LINEAR_PAGES, RawIssue, RawState,
    pages::{self, End, List},
    session::{Pager, Session},
};
use crate::{
    config::LinearWorkflow,
    domain::{
        Incompleteness, LinearTeamStates, SourceState,
        facts::{LinearIssue, LinearPriority, LinearState, LinearStateKind, LinearTeam},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

const TEAM_STATES_QUERY: &str = include_str!("queries/team_states.graphql");
const START_TEAM_ISSUES_QUERY: &str = include_str!("queries/start_team_issues.graphql");

/// Workflow states read per team. A team with more is read partially, and
/// its state names aren't checked.
pub const MAX_TEAM_STATES: u32 = 50;

/// Linear's priority numbers from urgent to low, read in this order when the
/// start teams' issues run past one page; every other priority (none, or a
/// number den doesn't know) comes last, as [`StartOrder`] ranks them.
///
/// [`StartOrder`]: crate::decide::StartOrder
const PRIORITY_BANDS: [i64; 4] = [1, 2, 3, 4];

/// The teams' workflow states, and the source's state: failed when the
/// config names a team or state Linear doesn't have.
pub(super) fn team_states(
    session: &Session<'_>,
    workflow: &LinearWorkflow,
) -> (Vec<LinearTeamStates>, SourceState) {
    let read = read_teams(session, workflow, &team_filter(workflow));
    let mut mismatches = Vec::new();
    let mut found = Vec::new();
    for key in &workflow.start_teams {
        match read
            .teams
            .iter()
            .find(|t| t.team.key.eq_ignore_ascii_case(key))
        {
            Some(team) => found.push(team),
            // The filter asks for every start team by key, so one that
            // isn't in the whole answer doesn't exist.
            None if read.in_full() => mismatches.push(format!(
                "start_teams names {key:?}, but the Linear workspace {} has no team with that key",
                workflow.workspace
            )),
            None => {}
        }
    }
    let mut read_state = read.state();
    if workflow.start_teams.is_empty() {
        mismatches.extend(missing_from_every_team(workflow, &read));
    } else if !found.is_empty() {
        mismatches.extend(missing_from_start_teams(workflow, &found, &read.truncated));
    } else if read.in_full() && workflow.state_names().next().is_some() {
        // No start team exists, so nothing holds the state names to a team:
        // they are checked against every team, so a typo in them shows now
        // and not a survey after the team key is fixed. Only on this broken
        // config does den read every team.
        let every = read_teams(session, workflow, &Value::Null);
        mismatches.extend(missing_from_every_team(workflow, &every));
        read_state = read_state.worst(every.state());
    }
    let state = if mismatches.is_empty() {
        read_state
    } else {
        SourceState::Failed {
            error: format!(
                "[linear.{}] names what Linear doesn't have: {}{}",
                workflow.owner,
                mismatches.join("; "),
                // What den couldn't read may hide more: say it, with any
                // rate limit's reset, rather than lose it behind the failure.
                match &read_state {
                    SourceState::Complete => String::new(),
                    incomplete => format!(
                        "; den's check was incomplete, so there may be more: {}",
                        match incomplete {
                            SourceState::Partial(why) => why.describe(),
                            other => other.describe(),
                        }
                    ),
                }
            ),
        }
    };
    (read.teams, state)
}

/// The teams one Linear Workflow read, and how far.
struct TeamsRead {
    teams: Vec<LinearTeamStates>,
    /// Teams whose states den couldn't read in full, by team ID.
    truncated: HashSet<String>,
    /// How the team list itself was read.
    list: SourceState,
}

impl TeamsRead {
    /// Whether every team the query asked for was read (an unreadable team
    /// node could be the one a name is looked for in).
    fn in_full(&self) -> bool {
        self.list == SourceState::Complete
    }

    /// The list's state, and partial for teams whose states den couldn't
    /// read in full.
    fn state(&self) -> SourceState {
        let mut keys: Vec<&str> = self
            .teams
            .iter()
            .filter(|t| self.truncated.contains(&t.team.id))
            .map(|t| t.team.key.as_str())
            .collect();
        keys.sort_unstable();
        match keys.first() {
            None => self.list.clone(),
            Some(first) => self
                .list
                .clone()
                .worst(SourceState::Partial(Incompleteness::FollowUpTruncated {
                    what: format!(
                        "the workflow states of {} team(s) beyond the first {MAX_TEAM_STATES}, first: {first}",
                        keys.len()
                    ),
                })),
        }
    }
}

/// Linear's `TeamFilter` for the start teams by key, any case (as
/// [`start_filter`] asks for their issues); null, every team, when the
/// workflow names none.
fn team_filter(workflow: &LinearWorkflow) -> Value {
    if workflow.start_teams.is_empty() {
        return Value::Null;
    }
    let keys: Vec<Value> = workflow
        .start_teams
        .iter()
        .map(|key| json!({ "key": { "eqIgnoreCase": key } }))
        .collect();
    json!({ "or": keys })
}

/// Every team `filter` matches, with its states, up to the page limit.
fn read_teams(session: &Session<'_>, workflow: &LinearWorkflow, filter: &Value) -> TeamsRead {
    let mut teams: Vec<LinearTeamStates> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut truncated: HashSet<String> = HashSet::new();
    let mut unreadable: Vec<String> = Vec::new();
    let mut pager = Pager::default();
    let list = List {
        operation: "LinearTeamStates",
        query: TEAM_STATES_QUERY,
        variables: |after: Option<&str>| {
            json!({
                "first": LINEAR_PAGE_SIZE,
                "after": after,
                "statesFirst": MAX_TEAM_STATES,
                "filter": filter,
            })
        },
        connection: |data: TeamsData| data.teams,
    };
    let end = pages::read_list(session, &mut pager, MAX_LINEAR_PAGES, &list, |node| {
        let raw: RawTeamStates = match serde_json::from_value(node) {
            Ok(raw) => raw,
            Err(e) => return unreadable.push(e.to_string()),
        };
        if !seen.insert(raw.id.clone()) {
            return;
        }
        if raw.states.page_info.has_next_page {
            truncated.insert(raw.id.clone());
        }
        teams.push(raw.into_team_states(workflow));
    });
    if end == End::Cut {
        let bound = pages::page_bound(&pager, None);
        pager.note(bound);
    }
    pages::note_unreadable(&mut pager, &unreadable, "Linear team(s)");
    TeamsRead {
        teams,
        truncated,
        list: pager.finish(),
    }
}

/// Each state name some found start team doesn't have (a team whose states
/// den couldn't read in full isn't checked).
fn missing_from_start_teams(
    workflow: &LinearWorkflow,
    found: &[&LinearTeamStates],
    truncated: &HashSet<String>,
) -> Vec<String> {
    let mut mismatches = Vec::new();
    for (setting, name) in workflow.state_names() {
        for team in found {
            if truncated.contains(&team.team.id) || has_state(&team.states, name) {
                continue;
            }
            mismatches.push(format!(
                "{setting} {name:?} is not a state of start team {}{}",
                team.team.key,
                spelling_hint(std::iter::once(*team), name)
            ));
        }
    }
    mismatches
}

/// Each state name no team in `read` has; none when den couldn't read every
/// team and all their states.
fn missing_from_every_team(workflow: &LinearWorkflow, read: &TeamsRead) -> Vec<String> {
    if !read.in_full() || !read.truncated.is_empty() {
        return Vec::new();
    }
    workflow
        .state_names()
        .filter(|(_, name)| !read.teams.iter().any(|t| has_state(&t.states, name)))
        .map(|(setting, name)| {
            format!(
                "{setting} {name:?} is not a state of any team in the Linear workspace {}{}",
                workflow.workspace,
                spelling_hint(read.teams.iter(), name)
            )
        })
        .collect()
}

/// " (Linear spells it ...)" when a state of `teams` matches `name` but for
/// case; empty otherwise.
fn spelling_hint<'t>(teams: impl IntoIterator<Item = &'t LinearTeamStates>, name: &str) -> String {
    teams
        .into_iter()
        .flat_map(|t| &t.states)
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .map(|s| format!(" (Linear spells it {:?})", s.name))
        .unwrap_or_default()
}

/// State names compare exactly, as a state move (#38) will look them up.
fn has_state(states: &[LinearState], name: &str) -> bool {
    states.iter().any(|s| s.name == name)
}

/// The start teams' unassigned, not-yet-started Linear Issues (see the
/// module docs), without those already in `known`.
///
/// One page, oldest first, holds them all in most workspaces. When Linear
/// has more, den reads them again by priority, urgent first and oldest first
/// within each, sharing one page bound: what the bound cuts is then the end
/// of the Start order (the newest of the least urgent), and the partial
/// reason says which priorities were read in full, cut, or not read. Within
/// a cut priority, issues in the active cycle aren't read first, though the
/// Start order puts them first. When a page is lost (an error or a rate
/// limit), the first page's issues are kept next to what was read by
/// priority, and the source is partial.
pub(super) fn start_team_issues(
    session: &Session<'_>,
    workflow: &LinearWorkflow,
    known: &[LinearIssue],
) -> (Vec<LinearIssue>, SourceState) {
    let filter = start_filter(workflow);
    let known: HashSet<&str> = known
        .iter()
        .filter(|i| i.id.workspace == workflow.workspace)
        .map(|i| i.id.id.as_str())
        .collect();
    let mut pager = Pager::default();

    let mut all = Collected::default();
    let end = pages::read_list(session, &mut pager, 1, &issues_list(&filter), |node| {
        all.take(node, workflow, &known);
    });
    if end != End::Cut {
        return all.finish(pager);
    }

    // More than one page: read again by priority. The first page's issues
    // are dropped, so that what is shown is exactly the head of the order,
    // unless a page is lost on the way (see below).
    let mut labels: Vec<String> = PRIORITY_BANDS
        .iter()
        .map(|&n| LinearPriority::from_linear(n).label())
        .collect();
    labels.push(LinearPriority::None.label());
    let mut comparators: Vec<Value> = PRIORITY_BANDS.iter().map(|n| json!({ "eq": n })).collect();
    comparators.push(json!({ "nin": PRIORITY_BANDS }));
    let mut by_priority = Collected::default();
    for (index, priority) in comparators.into_iter().enumerate() {
        let started = if pager.pages_read() >= MAX_LINEAR_PAGES {
            false
        } else {
            let mut filter = filter.clone();
            filter["priority"] = priority;
            let list = issues_list(&filter);
            match pages::read_list(session, &mut pager, MAX_LINEAR_PAGES, &list, |node| {
                by_priority.take(node, workflow, &known);
            }) {
                End::Done => continue,
                // The pager says why; the later priorities are left unread.
                // The head of the order is then unknown anyway, so the first
                // page's issues are kept rather than lost with it.
                End::Stopped => {
                    by_priority.fall_back_to(all);
                    break;
                }
                End::Cut => true,
            }
        };
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let bound = pages::page_bound(&pager, Some(&cut_description(&labels, index, started)));
        pager.note(bound);
        break;
    }
    by_priority.finish(pager)
}

/// Says which end of the start teams' issues den's page bound cut: the
/// priorities before `index` read in full, the one at `index` only its
/// oldest (when `started`) or not at all, and the rest not at all.
fn cut_description(labels: &[&str], index: usize, started: bool) -> String {
    let (full, rest) = labels.split_at(index);
    let (cut, unread) = if started {
        (rest.first().copied(), &rest[1..])
    } else {
        (None, rest)
    };
    let mut parts = vec!["read most urgent first and oldest first".to_owned()];
    if !full.is_empty() {
        parts.push(format!("{} in full", and_list(full)));
    }
    if let Some(cut) = cut {
        parts.push(format!("{cut} only its oldest (the newer ones not read)"));
    }
    if !unread.is_empty() {
        parts.push(format!("{} not read", and_list(unread)));
    }
    parts.join(", ")
}

/// "a", "a and b", "a, b and c".
fn and_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The start-team issues query for `filter`.
fn issues_list(
    filter: &Value,
) -> List<'static, impl Fn(Option<&str>) -> Value + '_, impl Fn(IssuesData) -> Connection> {
    List {
        operation: "LinearStartTeamIssues",
        query: START_TEAM_ISSUES_QUERY,
        variables: move |after: Option<&str>| json!({ "first": LINEAR_PAGE_SIZE, "after": after, "filter": filter }),
        connection: |data: IssuesData| data.issues,
    }
}

/// Linear Issues read so far, each once.
#[derive(Default)]
struct Collected {
    issues: Vec<LinearIssue>,
    seen: HashSet<String>,
    unreadable: Vec<String>,
}

impl Collected {
    fn take(&mut self, node: Value, workflow: &LinearWorkflow, known: &HashSet<&str>) {
        match serde_json::from_value::<RawIssue>(node) {
            Ok(raw) if known.contains(raw.id.as_str()) => {}
            Ok(raw) if self.seen.insert(raw.id.clone()) => {
                self.issues.push(raw.into_issue(workflow));
            }
            Ok(_) => {}
            Err(e) => self.unreadable.push(e.to_string()),
        }
    }

    /// Adds the issues of `earlier` not read already. Its unreadable nodes
    /// aren't added: they may be the same nodes read again, and the list is
    /// partial anyway.
    fn fall_back_to(&mut self, earlier: Self) {
        for issue in earlier.issues {
            if self.seen.insert(issue.id.id.clone()) {
                self.issues.push(issue);
            }
        }
    }

    fn finish(self, mut pager: Pager) -> (Vec<LinearIssue>, SourceState) {
        pages::note_unreadable(&mut pager, &self.unreadable, "Linear Issue(s)");
        (self.issues, pager.finish())
    }
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
