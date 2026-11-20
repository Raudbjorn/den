//! The Branch fetcher over a scripted GitHub: which Branches it lists, checks
//! and compares, what facts and coverage it reports, and what it asks GitHub.
//! Thin adapter checks; the real `gh` is checked by hand (spec, Testing Decisions).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{
        Observation, Repo, Source, SourceKind, SourceState, UpstreamTargets,
        facts::{Branch, ComparisonBranch},
    },
    github::{
        branches,
        graphql::{GraphQlRequest, GraphQlResponse, TransportError},
        session::{RateLimitMemo, Session},
        survey, survey_with,
    },
};
use serde_json::{Value, json};
use std::sync::Arc;
use support::{
    branch_graphql::{FakeBranch, FakeBranches},
    build::{branch::fork, now, repo_id},
    fakes::{ScriptedGraphQl, data, operation},
};

const FORKED_AT: &str = "2026-06-01T00:00:00Z";

fn repo_node(name: &str, is_fork: bool, default_branch: &str) -> Value {
    json!({
        "name": name,
        "url": format!("https://github.com/octo/{name}"),
        "isFork": is_fork,
        "isArchived": false,
        "defaultBranchRef": { "name": default_branch },
        "parent": if is_fork {
            json!({
                "name": name, "owner": { "login": "upstream" },
                "url": format!("https://github.com/upstream/{name}"), "isFork": false,
                "isArchived": false, "defaultBranchRef": { "name": "main" }
            })
        } else {
            Value::Null
        }
    })
}

/// Answers the viewer, Owner "octo"'s `repos` (no PRs), and the Branch queries
/// from `world`; `fail` may override any request first.
fn octo(
    repos: Vec<Value>,
    world: FakeBranches,
    fail: impl Fn(&GraphQlRequest) -> Option<Result<GraphQlResponse, TransportError>>
    + Send
    + Sync
    + 'static,
) -> ScriptedGraphQl {
    ScriptedGraphQl::new(move |request| {
        if let Some(answer) = fail(request) {
            return answer;
        }
        if let Some(answer) = world.answer(request) {
            return answer;
        }
        let empty_page = json!({ "hasNextPage": false, "endCursor": null });
        match operation(request).as_str() {
            "Viewer" => Ok(data(json!({ "viewer": { "login": "me" } }))),
            "ViewerTeams" => Ok(data(json!({ "viewer": { "organizations": {
                "pageInfo": empty_page, "nodes": [] } } }))),
            "OwnerRepos" => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": empty_page, "nodes": repos } } }))),
            "OwnerPullRequests" => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": empty_page, "nodes": [] } } }))),
            "OwnerIssueCounts" => Ok(data(json!({ "repositoryOwner": { "repositories": {
                "pageInfo": empty_page, "nodes": [] } } }))),
            other => panic!("unexpected {other}"),
        }
    })
}

fn no_failures(_: &GraphQlRequest) -> Option<Result<GraphQlResponse, TransportError>> {
    None
}

fn branch_named<'a>(observation: &'a Observation, repo: &str, name: &str) -> Option<&'a Branch> {
    observation
        .branches
        .iter()
        .find(|b| b.repo == repo_id(repo) && b.name == name)
}

fn names(observation: &Observation) -> Vec<String> {
    let mut names: Vec<String> = observation
        .branches
        .iter()
        .map(|b| format!("{}:{}", b.repo, b.name))
        .collect();
    names.sort();
    names
}

fn ops(gql: &ScriptedGraphQl, op: &str) -> Vec<GraphQlRequest> {
    gql.requests()
        .into_iter()
        .filter(|r| operation(r) == op)
        .collect()
}

fn coverage(observation: &Observation, source: &Source) -> Option<SourceState> {
    observation.coverage.get(source).cloned()
}

fn owner_branches() -> Source {
    Source::Owner {
        owner: "octo".to_owned(),
        kind: SourceKind::Branches,
    }
}

fn repo_branches(repo: &str) -> Source {
    Source::Repo {
        repo: repo_id(repo),
        kind: SourceKind::Branches,
    }
}

#[test]
fn only_candidates_are_checked_and_only_unproposed_ones_compared() {
    let world = FakeBranches::default().repo(
        "octo/app",
        "2025-01-01T00:00:00Z",
        vec![
            FakeBranch::new("main", "2026-11-18T00:00:00Z"),
            FakeBranch::new("had-a-pr", "2026-11-10T00:00:00Z").prs(2),
            FakeBranch::new("wip", "2026-11-12T00:00:00Z").ahead(3),
            FakeBranch::new("anon", "2026-09-01T00:00:00Z")
                .unmatched()
                .ahead(1),
        ],
    );
    let gql = octo(vec![repo_node("app", false, "main")], world, no_failures);
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");

    // The default branch is never even checked.
    assert_eq!(
        names(&observation),
        ["octo/app:anon", "octo/app:had-a-pr", "octo/app:wip"]
    );
    let checked: Vec<String> = ops(&gql, "BranchCandidates")
        .iter()
        .flat_map(|r| {
            r.variables
                .as_object()
                .expect("variables")
                .iter()
                .filter(|(k, _)| k.starts_with("ref"))
                .map(|(_, v)| v.as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        !checked.contains(&"refs/heads/main".to_owned()),
        "{checked:?}"
    );

    let had_pr = branch_named(&observation, "octo/app", "had-a-pr").expect("had-a-pr");
    assert!(had_pr.ever_had_pr);
    assert_eq!(had_pr.commits_ahead, None, "a PR head is never compared");

    let wip = branch_named(&observation, "octo/app", "wip").expect("wip");
    assert!(!wip.ever_had_pr);
    assert_eq!(wip.commits_ahead, Some(3));
    assert_eq!(wip.last_commit_author.as_deref(), Some("me"));
    assert_eq!(wip.last_commit_headline, "work on wip");
    assert_eq!(
        wip.last_commit_at,
        "2026-11-12T00:00:00Z".parse().expect("ts")
    );
    assert_eq!(wip.url, "https://github.com/octo/app/tree/wip");
    assert_eq!(
        wip.comparison,
        ComparisonBranch {
            repo: repo_id("octo/app"),
            name: "main".to_owned()
        }
    );

    let anon = branch_named(&observation, "octo/app", "anon").expect("anon");
    assert_eq!(anon.last_commit_author, None);

    let compared_heads: Vec<Value> = ops(&gql, "BranchComparisons")
        .iter()
        .flat_map(|r| {
            r.variables
                .as_object()
                .expect("variables")
                .iter()
                .filter(|(k, _)| k.starts_with("head"))
                .map(|(_, v)| v.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(compared_heads.len(), 2, "{compared_heads:?}");
    assert!(!compared_heads.contains(&json!("refs/heads/had-a-pr")));

    assert_eq!(
        coverage(&observation, &owner_branches()),
        Some(SourceState::Complete)
    );
    assert_eq!(
        coverage(&observation, &repo_branches("octo/app")),
        Some(SourceState::Complete),
        "explicit Repo completion lets a refresh remove deleted branches"
    );
    // Values travel as variables; only alias indices are generated into the text.
    assert!(
        gql.requests()
            .iter()
            .all(|r| !r.query.contains("octo") && !r.query.contains("wip"))
    );
}

#[test]
fn a_forks_branches_are_compared_against_its_own_default_and_those_it_came_with_are_skipped() {
    let world = FakeBranches::default().repo(
        "octo/tool",
        FORKED_AT,
        vec![
            FakeBranch::new("trunk", "2026-11-01T00:00:00Z"),
            // Came with the fork: last commit before the fork was created.
            FakeBranch::new("upstream-feature", "2026-05-01T00:00:00Z").ahead(40),
            FakeBranch::new("my-fix", "2026-11-15T00:00:00Z").ahead(2),
        ],
    );
    let gql = octo(vec![repo_node("tool", true, "trunk")], world, no_failures);
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");

    assert_eq!(names(&observation), ["octo/tool:my-fix"]);
    let fix = branch_named(&observation, "octo/tool", "my-fix").expect("my-fix");
    assert_eq!(fix.commits_ahead, Some(2));
    assert_eq!(
        fix.comparison,
        ComparisonBranch {
            repo: repo_id("octo/tool"),
            name: "trunk".to_owned()
        }
    );
    let compare = ops(&gql, "BranchComparisons");
    assert_eq!(compare.len(), 1);
    let vars = &compare[0].variables;
    assert_eq!(
        (
            &vars["owner0"],
            &vars["name0"],
            &vars["base0"],
            &vars["head0"]
        ),
        (
            &json!("octo"),
            &json!("tool"),
            &json!("refs/heads/trunk"),
            &json!("refs/heads/my-fix")
        ),
        "compared inside the fork, never against upstream"
    );
    assert!(
        gql.requests()
            .iter()
            .all(|r| !r.variables.to_string().contains("upstream"))
    );
}

#[test]
fn a_repo_with_many_branches_is_read_from_both_ends_without_gaps_or_duplicates() {
    let many: Vec<FakeBranch> = (0..250)
        .map(|i| FakeBranch::new(&format!("b{i:03}"), "2026-11-10T00:00:00Z").prs(1))
        .chain([FakeBranch::new("main", "2026-11-10T00:00:00Z")])
        .collect();
    let world = FakeBranches::default().repo("octo/big", "2025-01-01T00:00:00Z", many);
    let gql = octo(vec![repo_node("big", false, "main")], world, no_failures);
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");

    let names = names(&observation);
    assert_eq!(names.len(), 250, "every Branch but main, once");
    assert_eq!(
        coverage(&observation, &repo_branches("octo/big")),
        Some(SourceState::Complete),
        "read completely, even if the Owner's Repo list is partial"
    );
    // 251 Branches at 100 per page: page 1 forwards, then both ends at once.
    let rounds = ops(&gql, "BranchRefs").len();
    assert!(rounds <= 3, "{rounds} list queries");
}

#[test]
fn a_lost_candidate_query_leaves_those_branches_out_and_says_so_for_their_repo() {
    let world = FakeBranches::default()
        .repo(
            "octo/app",
            "2025-01-01T00:00:00Z",
            vec![
                FakeBranch::new("main", "2026-11-10T00:00:00Z"),
                FakeBranch::new("wip", "2026-11-10T00:00:00Z"),
            ],
        )
        .repo(
            "octo/lib",
            "2025-01-01T00:00:00Z",
            vec![FakeBranch::new("main", "2026-11-10T00:00:00Z")],
        );
    let gql = octo(
        vec![
            repo_node("app", false, "main"),
            repo_node("lib", false, "main"),
        ],
        world,
        |request| {
            (operation(request) == "BranchCandidates").then(|| {
                Err(TransportError::TimedOut {
                    after: std::time::Duration::from_secs(30),
                })
            })
        },
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert!(observation.branches.is_empty(), "nothing is guessed");
    match coverage(&observation, &repo_branches("octo/app")) {
        Some(SourceState::Partial(why)) => {
            let text = why.describe();
            assert!(text.contains("wip") && text.contains("30s"), "{text}");
        }
        other => panic!("expected partial coverage for octo/app, got {other:?}"),
    }
    // A successful Repo keeps explicit coverage even when other Repos are partial.
    assert_eq!(
        coverage(&observation, &repo_branches("octo/lib")),
        Some(SourceState::Complete)
    );
}

#[test]
fn a_repo_github_cannot_read_fails_its_own_branches_only() {
    // "octo/gone" is listed as a Repo but GitHub answers null for it.
    let world = FakeBranches::default().repo(
        "octo/app",
        "2025-01-01T00:00:00Z",
        vec![
            FakeBranch::new("main", "2026-11-10T00:00:00Z"),
            FakeBranch::new("wip", "2026-11-10T00:00:00Z"),
        ],
    );
    let gql = octo(
        vec![
            repo_node("app", false, "main"),
            repo_node("gone", false, "main"),
        ],
        world,
        no_failures,
    );
    let observation = survey(&gql, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(names(&observation), ["octo/app:wip"]);
    assert!(matches!(
        coverage(&observation, &repo_branches("octo/gone")),
        Some(SourceState::Failed { error }) if error.contains("octo/gone")
    ));
    assert_eq!(
        coverage(&observation, &owner_branches()),
        Some(SourceState::Complete)
    );
}

#[test]
fn the_comparison_branch_is_a_parameter_and_another_repos_branch_is_compared_across() {
    // What #13 does once an Upstream is set: compare in the parent, head `owner:branch`.
    let world = FakeBranches::default()
        .repo(
            "octo/tool",
            FORKED_AT,
            vec![
                FakeBranch::new("trunk", "2026-11-01T00:00:00Z"),
                FakeBranch::new("my-fix", "2026-11-15T00:00:00Z").ahead(5),
            ],
        )
        .repo(
            "upstream/tool",
            "2020-01-01T00:00:00Z",
            vec![FakeBranch::new("main", "2026-11-01T00:00:00Z")],
        );
    let gql = Arc::new(octo(Vec::new(), world, no_failures));
    let memo = RateLimitMemo::default();
    let session = Session::new(gql.as_ref(), &memo);
    let repos: Vec<Repo> = vec![fork("octo/tool", "upstream/tool", "trunk")];
    let upstream = |repo: &Repo| {
        repo.parent.as_ref().map(|parent| ComparisonBranch {
            repo: parent.id.clone(),
            name: "main".to_owned(),
        })
    };
    let fetched = branches::fetch_against(&session, "octo", &repos, &upstream).expect("fetch");
    let [fix] = fetched.items.as_slice() else {
        panic!("{:?}", fetched.items)
    };
    assert_eq!(fix.name, "my-fix");
    assert_eq!(fix.commits_ahead, Some(5));
    assert_eq!(fix.comparison.repo, repo_id("upstream/tool"));
    let compare = ops(&gql, "BranchComparisons");
    let vars = &compare[0].variables;
    assert_eq!(
        (&vars["owner0"], &vars["name0"], &vars["head0"]),
        (&json!("upstream"), &json!("tool"), &json!("octo:my-fix"))
    );
}

#[test]
fn a_survey_compares_a_fork_whose_target_is_its_upstream_against_the_upstreams_default_branch() {
    // #13: the survey takes the Upstream choices; only the chosen fork is
    // compared across Repos, as `octo:<branch>` in the Upstream.
    let world = FakeBranches::default()
        .repo(
            "octo/tool",
            FORKED_AT,
            vec![
                FakeBranch::new("trunk", "2026-11-01T00:00:00Z"),
                FakeBranch::new("my-fix", "2026-11-15T00:00:00Z").ahead(4),
            ],
        )
        .repo(
            "octo/other",
            FORKED_AT,
            vec![
                FakeBranch::new("trunk", "2026-11-01T00:00:00Z"),
                FakeBranch::new("wip", "2026-11-15T00:00:00Z").ahead(1),
            ],
        )
        .repo(
            "upstream/tool",
            "2020-01-01T00:00:00Z",
            vec![FakeBranch::new("main", "2026-11-01T00:00:00Z")],
        );
    let gql = octo(
        vec![
            repo_node("tool", true, "trunk"),
            repo_node("other", true, "trunk"),
        ],
        world,
        no_failures,
    );
    let upstreams = UpstreamTargets::new([repo_id("octo/tool")]);
    let observation = survey_with(&gql, &["octo".to_owned()], &upstreams, now()).expect("a survey");

    let fix = branch_named(&observation, "octo/tool", "my-fix").expect("my-fix");
    assert_eq!(
        fix.comparison,
        ComparisonBranch {
            repo: repo_id("upstream/tool"),
            name: "main".to_owned()
        }
    );
    let wip = branch_named(&observation, "octo/other", "wip").expect("wip");
    assert_eq!(
        wip.comparison,
        ComparisonBranch {
            repo: repo_id("octo/other"),
            name: "trunk".to_owned()
        },
        "a fork not chosen stays standalone"
    );
    let heads: Vec<Value> = ops(&gql, "BranchComparisons")
        .iter()
        .flat_map(|r| {
            r.variables
                .as_object()
                .into_iter()
                .flat_map(|vars| {
                    vars.iter()
                        .filter(|(k, _)| k.starts_with("head"))
                        .map(|(_, v)| v.clone())
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(heads.contains(&json!("octo:my-fix")), "{heads:?}");
    assert!(heads.contains(&json!("refs/heads/wip")), "{heads:?}");
}
