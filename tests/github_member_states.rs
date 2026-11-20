//! The GitHub source's read of Landing Group members' states (#37), over a
//! scripted transport: aliased `repository { pullRequest(number:) }` per
//! member, Owners, Repos and numbers as variables, batches of
//! `MEMBERS_PER_QUERY`, at most `MAX_MEMBER_STATES` members, and honest
//! coverage per Owner.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{
        Incompleteness, RepoId, Source, SourceKind, SourceState,
        facts::{MemberPullRequestState, PullRequestRef, PullRequestState},
    },
    github::{
        GhSource,
        graphql::{GraphQl, GraphQlError, GraphQlRequest, GraphQlResponse, TransportError},
        member_states::{MAX_MEMBER_STATES, MEMBERS_PER_QUERY},
    },
    ports::{GitHubSource, MemberStatesRequest},
};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use support::{
    build::ts,
    fakes::{ScriptedGraphQl, data},
};

/// A merged member, a closed one, one open again, one GitHub has no Pull
/// Request for, and one in a state den doesn't know.
const MERGED: u64 = 1;
const CLOSED: u64 = 2;
const OPEN: u64 = 3;
const MISSING: u64 = 404;
const STRANGE: u64 = 77;

fn member(repo: &str, number: u64) -> PullRequestRef {
    let (owner, name) = repo.split_once('/').unwrap();
    PullRequestRef::new(RepoId::new(owner, name), number)
}

/// Answers each alias `m<i>` by the number in `p<i>`.
fn answer(request: &GraphQlRequest) -> Map<String, Value> {
    let variables = request.variables.as_object().unwrap();
    let mut out = Map::new();
    for i in 0.. {
        let Some(number) = variables.get(&format!("p{i}")).and_then(Value::as_u64) else {
            break;
        };
        let node = match number {
            MERGED => {
                json!({ "pullRequest": { "state": "MERGED", "mergedAt": "2026-11-18T09:00:00Z" } })
            }
            CLOSED => json!({ "pullRequest": { "state": "CLOSED", "mergedAt": null } }),
            MISSING => json!({ "pullRequest": null }),
            STRANGE => json!({ "pullRequest": { "state": "WEIRD", "mergedAt": null } }),
            _ => json!({ "pullRequest": { "state": "OPEN", "mergedAt": null } }),
        };
        out.insert(format!("m{i}"), node);
    }
    out
}

fn source(owner: &str) -> Source {
    Source::Owner {
        owner: owner.to_owned(),
        kind: SourceKind::MemberStates,
    }
}

fn read_with(
    transport: Arc<ScriptedGraphQl>,
    members: Vec<PullRequestRef>,
) -> den::ports::MemberStatesRead {
    struct Shared(Arc<ScriptedGraphQl>);
    impl GraphQl for Shared {
        fn execute(&self, request: &GraphQlRequest) -> Result<GraphQlResponse, TransportError> {
            self.0.execute(request)
        }
    }
    GhSource::new(Shared(transport)).member_states(&MemberStatesRequest {
        pull_requests: members,
    })
}

#[test]
fn reads_each_members_state_by_owner_repo_and_number() {
    let transport = Arc::new(ScriptedGraphQl::new(|r| Ok(data(Value::Object(answer(r))))));
    let read = read_with(
        transport.clone(),
        vec![
            member("acme/api", MERGED),
            member("acme/api", CLOSED),
            member("acme/app", OPEN),
            member("octo/api", MERGED),
        ],
    );
    assert_eq!(
        read.states,
        [
            MemberPullRequestState {
                pr: member("acme/api", MERGED),
                state: PullRequestState::Merged {
                    merged_at: Some(ts("2026-11-18T09:00:00Z")),
                },
            },
            MemberPullRequestState {
                pr: member("acme/api", CLOSED),
                state: PullRequestState::Closed,
            },
            MemberPullRequestState {
                pr: member("acme/app", OPEN),
                state: PullRequestState::Open,
            },
            MemberPullRequestState {
                pr: member("octo/api", MERGED),
                state: PullRequestState::Merged {
                    merged_at: Some(ts("2026-11-18T09:00:00Z")),
                },
            },
        ]
    );
    assert_eq!(
        read.coverage.get(&source("acme")),
        Some(&SourceState::Complete)
    );
    assert_eq!(
        read.coverage.get(&source("octo")),
        Some(&SourceState::Complete)
    );

    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    let query = &requests[0].query;
    assert!(query.starts_with("query MemberStates("), "{query}");
    assert!(
        query.contains("m0: repository(owner: $o0, name: $n0)")
            && query.contains("pullRequest(number: $p0)")
            && query.contains("$p3: Int!"),
        "{query}"
    );
    assert!(
        !query.contains("acme"),
        "names travel as variables: {query}"
    );
    assert_eq!(requests[0].variables["o0"], "acme");
    assert_eq!(requests[0].variables["n2"], "app");
    assert_eq!(requests[0].variables["p1"], CLOSED);
}

/// A member GitHub returned no Pull Request (or an unknown state) for gets
/// no state, and its Owner's coverage says which; errors next to data make
/// the batch's Owners partial.
#[test]
fn a_member_den_got_no_state_for_is_partial_never_guessed() {
    let transport = Arc::new(ScriptedGraphQl::new(|r| {
        Ok(GraphQlResponse {
            data: Some(Value::Object(answer(r))),
            errors: vec![GraphQlError {
                message: "Could not resolve to a PullRequest with the number of 404.".to_owned(),
                kind: Some("NOT_FOUND".to_owned()),
            }],
        })
    }));
    let read = read_with(
        transport,
        vec![
            member("acme/api", MERGED),
            member("acme/api", MISSING),
            member("acme/api", STRANGE),
        ],
    );
    assert_eq!(read.states.len(), 1, "{:?}", read.states);
    let state = read.coverage.get(&source("acme")).unwrap();
    assert!(!state.is_complete());
    let text = state.describe();
    assert!(text.contains("Could not resolve"), "{text}");
}

#[test]
fn missing_and_unknown_states_alone_make_the_owner_partial() {
    let transport = Arc::new(ScriptedGraphQl::new(|r| Ok(data(Value::Object(answer(r))))));
    let read = read_with(
        transport,
        vec![member("acme/api", MISSING), member("acme/api", STRANGE)],
    );
    assert!(read.states.is_empty());
    let text = read.coverage.get(&source("acme")).unwrap().describe();
    assert!(
        text.contains("the state of acme/api #404") && text.contains("no such Pull Request"),
        "{text}"
    );
}

/// At most `MAX_MEMBER_STATES` members, in batches of `MEMBERS_PER_QUERY`;
/// the rest are recorded as not read.
#[test]
fn members_beyond_the_bound_are_not_read_and_say_so() {
    let transport = Arc::new(ScriptedGraphQl::new(|r| Ok(data(Value::Object(answer(r))))));
    let members: Vec<PullRequestRef> = (1000..1000 + MAX_MEMBER_STATES as u64 + 3)
        .map(|n| member("acme/api", n))
        .collect();
    let read = read_with(transport.clone(), members);
    assert_eq!(read.states.len(), MAX_MEMBER_STATES);
    assert_eq!(
        transport.requests().len(),
        MAX_MEMBER_STATES.div_ceil(MEMBERS_PER_QUERY)
    );
    match read.coverage.get(&source("acme")) {
        Some(SourceState::Partial(Incompleteness::FollowUpTruncated { what })) => {
            assert!(what.contains("den reads at most 100"), "{what}");
        }
        other => panic!("{other:?}"),
    }
}

/// A rate limit stays a rate limit; gh missing fails every Owner asked
/// about, and nothing more is sent.
#[test]
fn transport_trouble_is_coverage() {
    let limited = Arc::new(ScriptedGraphQl::new(|_| {
        Err(TransportError::RateLimited {
            detail: "API rate limit exceeded".to_owned(),
        })
    }));
    let read = read_with(limited, vec![member("acme/api", MERGED)]);
    assert!(read.states.is_empty());
    assert!(matches!(
        read.coverage.get(&source("acme")),
        Some(SourceState::Partial(Incompleteness::RateLimited { .. }))
    ));

    let missing = Arc::new(ScriptedGraphQl::new(|_| {
        Err(TransportError::GhNotInstalled)
    }));
    let members: Vec<PullRequestRef> = (1..=MEMBERS_PER_QUERY as u64 + 1)
        .map(|n| member("acme/api", n + 10))
        .chain([member("octo/api", MERGED)])
        .collect();
    let read = read_with(missing.clone(), members);
    assert_eq!(missing.requests().len(), 1, "stops after gh is missing");
    assert!(matches!(
        read.coverage.get(&source("acme")),
        Some(SourceState::Failed { .. })
    ));
    assert!(
        read.coverage
            .get(&source("octo"))
            .is_some_and(|s| !s.is_complete()),
        "{:?}",
        read.coverage
    );
}

/// When `gh` fails on one batch, the batches never sent fail with the same
/// error: they weren't read because `gh` failed, not because of den's bound,
/// so an Owner only in a later batch reads failed, with the real cause.
#[test]
fn batches_not_sent_after_gh_fails_carry_its_error_not_the_bound() {
    let logged_out = Arc::new(ScriptedGraphQl::new(|_| {
        Err(TransportError::NotLoggedIn {
            detail: "run gh auth login".to_owned(),
        })
    }));
    let members: Vec<PullRequestRef> = (1..=MEMBERS_PER_QUERY as u64)
        .map(|n| member("acme/api", n + 10))
        .chain((1..=15).map(|n| member("beta/api", n + 10)))
        .collect();
    let read = read_with(logged_out.clone(), members);
    assert_eq!(logged_out.requests().len(), 1, "stops after gh fails");
    match read.coverage.get(&source("beta")) {
        Some(SourceState::Failed { error }) => {
            assert!(error.contains("not logged in"), "{error}");
            assert!(!error.contains("at most"), "{error}");
        }
        other => panic!("beta: {other:?}"),
    }
}

/// `asked` holds exactly the members den sent to GitHub: within the bound,
/// and only the batches sent before `gh` failed. Those are the ones a
/// missing state counts against (they rotate behind the rest next time).
#[test]
fn asked_names_the_members_sent_and_no_others() {
    let transport = Arc::new(ScriptedGraphQl::new(|r| Ok(data(Value::Object(answer(r))))));
    let members: Vec<PullRequestRef> = (1000..1000 + MAX_MEMBER_STATES as u64 + 3)
        .map(|n| member("acme/api", n))
        .collect();
    let read = read_with(transport, members.clone());
    assert_eq!(read.asked, members[..MAX_MEMBER_STATES]);

    let logged_out = Arc::new(ScriptedGraphQl::new(|_| {
        Err(TransportError::NotLoggedIn {
            detail: "run gh auth login".to_owned(),
        })
    }));
    let members: Vec<PullRequestRef> = (1..=MEMBERS_PER_QUERY as u64 + 5)
        .map(|n| member("acme/api", n + 10))
        .collect();
    let read = read_with(logged_out, members.clone());
    assert_eq!(read.asked, members[..MEMBERS_PER_QUERY]);
}
