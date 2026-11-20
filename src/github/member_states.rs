//! Landing Group members' states (#37): the Pull Requests attached to a
//! Linear Issue that weren't among their Repo's open Pull Requests. The survey
//! reads open Pull Requests only, so it can't tell a merged member from one
//! closed without merging; Mark for release needs to. den asks for them by
//! Owner, Repo and number, read-only, after both surveys.
//!
//! Bounded: at most [`MAX_MEMBER_STATES`] members per read, in batches of
//! [`MEMBERS_PER_QUERY`] (one aliased `repository { pullRequest(number:) }`
//! per member), each call with `gh`'s timeout. Coverage is honest, per Owner
//! (`SourceKind::MemberStates`): a lost batch, errors next to data, a member
//! GitHub returned nothing (or an unknown state) for, a rate limit, or members
//! beyond the bound make that Owner's entry incomplete, and say which. A
//! member without a state is never given one.

use super::{
    graphql::GraphQl,
    session::{Fetched, RateLimitMemo, Session},
};
use crate::{
    domain::{
        Incompleteness, Source, SourceKind, SourceState,
        facts::{MemberPullRequestState, PullRequestRef, PullRequestState},
        ids::same_login,
    },
    ports::{MemberStatesRead, MemberStatesRequest},
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Map, Value};

/// Members per query. Each asks for two scalar fields, so a batch costs
/// GitHub little; the bound keeps one call's answer small.
pub const MEMBERS_PER_QUERY: usize = 25;
/// The most members den reads the state of after one survey. More are
/// recorded as not read (partial), never guessed.
pub const MAX_MEMBER_STATES: usize = 100;

const SELECTION: &str = include_str!("queries/member_state.graphql");
const NUMBER_PLACEHOLDER: &str = "$NUMBER";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoNode {
    pull_request: Option<PullRequestNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestNode {
    state: String,
    merged_at: Option<Timestamp>,
}

/// Reads the states of `request`'s members (see the module docs).
pub fn read(gql: &dyn GraphQl, request: &MemberStatesRequest) -> MemberStatesRead {
    let rate_limit = RateLimitMemo::default();
    let session = Session::new(gql, &rate_limit);
    let mut found: Vec<MemberPullRequestState> = Vec::new();
    let mut owners = OwnerStates::default();
    for pr in &request.pull_requests {
        owners.note(&pr.repo.owner, SourceState::Complete);
    }
    let bound = request.pull_requests.len().min(MAX_MEMBER_STATES);
    let (asked, beyond) = request.pull_requests.split_at(bound);
    let mut sent: Vec<PullRequestRef> = Vec::new();
    let mut batches = asked.chunks(MEMBERS_PER_QUERY);
    // Set when gh failed: why every batch not yet sent goes unread.
    let mut stopped: Option<SourceState> = None;
    for batch in batches.by_ref() {
        sent.extend(batch.iter().cloned());
        match session.run::<Map<String, Value>>(&batch_query(batch.len()), variables(batch)) {
            Err(failure) => {
                // gh missing or logged out: nothing more can be read.
                let failed = SourceState::Failed {
                    error: failure.to_string(),
                };
                for pr in batch {
                    owners.note(&pr.repo.owner, failed.clone());
                }
                stopped = Some(failed);
                break;
            }
            Ok(Fetched::Lost(state)) => {
                for pr in batch {
                    owners.note(&pr.repo.owner, lost(&state, pr));
                }
            }
            Ok(Fetched::Data { data, state }) => {
                for (i, pr) in batch.iter().enumerate() {
                    if !state.is_complete() {
                        owners.note(&pr.repo.owner, state.clone());
                    }
                    let node = data.get(&format!("m{i}")).cloned().unwrap_or(Value::Null);
                    match decode(node) {
                        Ok(member) => found.push(MemberPullRequestState {
                            pr: pr.clone(),
                            state: member,
                        }),
                        Err(why) => owners.note(
                            &pr.repo.owner,
                            SourceState::Partial(Incompleteness::FollowUpLost {
                                what: format!("the state of {} #{}", pr.repo, pr.number),
                                reason: why,
                            }),
                        ),
                    }
                }
            }
        }
    }
    // Batches never sent fail as gh did; members beyond the bound weren't
    // asked about at all.
    if let Some(failed) = &stopped {
        for pr in batches.flatten() {
            owners.note(&pr.repo.owner, failed.clone());
        }
    }
    for pr in beyond {
        owners.note(
            &pr.repo.owner,
            SourceState::Partial(Incompleteness::FollowUpTruncated {
                what: format!(
                    "the state of {} #{} (den reads at most {MAX_MEMBER_STATES} members' states after a survey)",
                    pr.repo, pr.number
                ),
            }),
        );
    }
    let mut read = MemberStatesRead {
        states: found,
        asked: sent,
        ..MemberStatesRead::default()
    };
    for (owner, state) in owners.0 {
        tracing::info!(owner = %owner, state = %state.describe(), "Landing Group member states");
        read.coverage.record(
            Source::Owner {
                owner,
                kind: SourceKind::MemberStates,
            },
            state,
        );
    }
    read
}

/// Each Owner asked about, in request order, with the worst state so far.
#[derive(Default)]
struct OwnerStates(Vec<(String, SourceState)>);

impl OwnerStates {
    fn note(&mut self, owner: &str, state: SourceState) {
        match self.0.iter_mut().find(|(o, _)| same_login(o, owner)) {
            Some((_, existing)) => {
                let previous = std::mem::replace(existing, SourceState::Complete);
                *existing = previous.worst(state);
            }
            None => self.0.push((owner.to_owned(), state)),
        }
    }
}

/// A lost batch, for one of its members: a rate limit stays a rate limit
/// (it says when it resets); anything else is a lost follow-up.
fn lost(state: &SourceState, pr: &PullRequestRef) -> SourceState {
    match state {
        SourceState::Partial(Incompleteness::RateLimited { .. }) => state.clone(),
        other => SourceState::Partial(Incompleteness::FollowUpLost {
            what: format!("the state of {} #{}", pr.repo, pr.number),
            reason: other.describe(),
        }),
    }
}

/// One member's node: its state, or why den has none.
fn decode(node: Value) -> Result<PullRequestState, String> {
    let repo = serde_json::from_value::<Option<RepoNode>>(node)
        .map_err(|e| format!("GitHub's response had an unexpected shape: {e}"))?
        .ok_or_else(|| "GitHub returned no such Repo".to_owned())?;
    let pr = repo
        .pull_request
        .ok_or_else(|| "GitHub returned no such Pull Request".to_owned())?;
    PullRequestState::from_github(&pr.state, pr.merged_at)
        .ok_or_else(|| format!("GitHub reported a state den doesn't know: {}", pr.state))
}

fn variables(batch: &[PullRequestRef]) -> Value {
    let mut variables = Map::new();
    for (i, pr) in batch.iter().enumerate() {
        variables.insert(format!("o{i}"), Value::String(pr.repo.owner.clone()));
        variables.insert(format!("n{i}"), Value::String(pr.repo.name.clone()));
        variables.insert(format!("p{i}"), Value::from(pr.number));
    }
    Value::Object(variables)
}

/// The query for a batch of `len` members. Only digits are spliced into the
/// text; Owners, Repos and numbers travel as variables.
fn batch_query(len: usize) -> String {
    let selection: String = SELECTION
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let declarations: Vec<String> = (0..len)
        .map(|i| format!("$o{i}: String!, $n{i}: String!, $p{i}: Int!"))
        .collect();
    let mut query = format!(
        "query MemberStates({}) {{\n  rateLimit {{ remaining resetAt }}\n",
        declarations.join(", ")
    );
    for i in 0..len {
        query.push_str(&format!(
            "  m{i}: repository(owner: $o{i}, name: $n{i}) {{\n{}\n  }}\n",
            selection.replace(NUMBER_PLACEHOLDER, &format!("$p{i}"))
        ));
    }
    query.push_str("}\n");
    query
}
