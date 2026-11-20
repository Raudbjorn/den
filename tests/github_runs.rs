//! The Runs fetcher over a scripted REST transport (which Repos are read, how
//! per-workflow lookups fill in a busy Repo, partial and failed coverage, rate
//! limits), and `gh api -i` as the REST transport against fake `gh` scripts.
//!
//! Thin adapter checks; the real `gh` is checked by hand (spec, Testing Decisions).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    domain::{
        CoverageEntry, Incompleteness, Repo, Source, SourceKind, SourceState, WorkflowId,
        facts::{Run, RunConclusion, RunStatus},
    },
    github::{
        graphql::{GhCli, GraphQl, GraphQlError, GraphQlRequest, GraphQlResponse, TransportError},
        rest::{Rest, RestResponse},
        runs,
        session::{KindFetch, RateLimitMemo, Session},
        survey,
    },
    ports::SurveyFailure,
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, sync::Mutex, time::Duration};
use support::{
    branch_graphql::only_default_branches,
    build::{now, repo_id, ts},
    fakes::{data, operation},
};

const RESET_EPOCH_SECS: i64 = 1_795_136_400;

type RestScript = dyn Fn(&str) -> Result<RestResponse, TransportError> + Send + Sync;

/// A transport with both sides: GraphQL answers the viewer and the Repo list,
/// and the other kinds' first queries with nothing found (anything else gets
/// empty data); REST answers from `script` by path.
struct ScriptedGitHub {
    repos: Value,
    /// An error GitHub sends alongside the Repo list's data.
    repo_list_error: Option<GraphQlError>,
    script: Box<RestScript>,
    paths: Mutex<Vec<String>>,
}

impl ScriptedGitHub {
    fn new(repos: Value, script: impl Fn(&str) -> RestResponse + Send + Sync + 'static) -> Self {
        Self::fallible(repos, move |path| Ok(script(path)))
    }

    /// REST answers that may be transport errors (no HTTP response at all).
    fn fallible(
        repos: Value,
        script: impl Fn(&str) -> Result<RestResponse, TransportError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            repos,
            repo_list_error: None,
            script: Box::new(script),
            paths: Mutex::default(),
        }
    }

    fn paths(&self) -> Vec<String> {
        let mut paths = self.paths.lock().expect("lock").clone();
        paths.sort();
        paths
    }
}

impl GraphQl for ScriptedGitHub {
    fn execute(&self, request: &GraphQlRequest) -> Result<GraphQlResponse, TransportError> {
        let operation = operation(request);
        let errors = match (operation.as_str(), &self.repo_list_error) {
            ("OwnerRepos", Some(error)) => vec![error.clone()],
            _ => Vec::new(),
        };
        if let Some(branches) = only_default_branches(request) {
            return branches;
        }
        let response = data(match operation.as_str() {
            "Viewer" => json!({ "viewer": { "login": "me" } }),
            "ViewerTeams" => json!({ "viewer": { "organizations": {
                "pageInfo": { "hasNextPage": false }, "nodes": []
            }}}),
            "OwnerRepos" => json!({ "repositoryOwner": { "repositories": {
                "pageInfo": { "hasNextPage": false }, "nodes": self.repos
            }}}),
            // Other kinds: nothing open, so the survey reads every kind completely.
            "OwnerPullRequests" | "OwnerIssueCounts" => {
                json!({ "repositoryOwner": { "repositories": {
                    "pageInfo": { "hasNextPage": false }, "nodes": []
                }}})
            }
            _ => json!({}),
        });
        Ok(GraphQlResponse { errors, ..response })
    }

    fn rest(&self) -> Option<&dyn Rest> {
        Some(self)
    }
}

impl Rest for ScriptedGitHub {
    fn get(&self, path: &str) -> Result<RestResponse, TransportError> {
        self.paths.lock().expect("lock").push(path.to_owned());
        (self.script)(path)
    }
}

fn ok(body: Value) -> RestResponse {
    RestResponse {
        status: 200,
        rate_limit_remaining: Some(4000),
        rate_limit_reset: None,
        body: Some(body),
    }
}

fn http(status: u16, message: &str) -> RestResponse {
    RestResponse {
        status,
        rate_limit_remaining: Some(4000),
        rate_limit_reset: None,
        body: Some(json!({ "message": message })),
    }
}

fn rate_limited() -> RestResponse {
    RestResponse {
        status: 403,
        rate_limit_remaining: Some(0),
        rate_limit_reset: jiff::Timestamp::from_second(RESET_EPOCH_SECS).ok(),
        body: Some(json!({ "message": "API rate limit exceeded for user ID 1." })),
    }
}

/// A Run node as GitHub's REST API sends it (the fields den reads).
fn run_node(repo: &str, workflow: (u64, &str), id: u64, conclusion: &str) -> Value {
    json!({
        "id": id,
        "name": workflow.1,
        "workflow_id": workflow.0,
        "path": format!(".github/workflows/{}.yml", workflow.1),
        "run_attempt": 1,
        "html_url": format!("https://github.com/{repo}/actions/runs/{id}"),
        "head_sha": format!("{id:040x}"),
        "head_branch": "main",
        "event": "push",
        "status": "completed",
        "conclusion": conclusion,
        "actor": { "login": "alice" },
        "triggering_actor": { "login": "alice" },
        "head_repository": { "full_name": repo },
        "run_started_at": "2026-11-18T11:00:00Z",
        "updated_at": "2026-11-18T12:00:00Z"
    })
}

fn with(mut node: Value, changes: Value) -> Value {
    for (key, value) in changes.as_object().expect("an object") {
        node[key] = value.clone();
    }
    node
}

fn runs_page(total: u64, runs: Vec<Value>) -> Value {
    json!({ "total_count": total, "workflow_runs": runs })
}

fn repo_node(name: &str, default_branch: Option<&str>, archived: bool) -> Value {
    json!({
        "name": name,
        "url": format!("https://github.com/octo/{name}"),
        "isFork": false,
        "isArchived": archived,
        "defaultBranchRef": default_branch.map(|b| json!({ "name": b })),
        "parent": null
    })
}

fn repo(name: &str, default_branch: Option<&str>, archived: bool) -> Repo {
    Repo {
        id: repo_id(&format!("octo/{name}")),
        url: format!("https://github.com/octo/{name}"),
        is_fork: false,
        is_archived: archived,
        default_branch: default_branch.map(str::to_owned),
        parent: None,
    }
}

fn fetch(github: &ScriptedGitHub, repos: &[Repo]) -> Result<KindFetch<Run>, SurveyFailure> {
    let memo = RateLimitMemo::default();
    runs::fetch(&Session::new(github, &memo), "octo", repos)
}

fn owner_state(fetched: &KindFetch<Run>) -> &SourceState {
    &fetched
        .coverage
        .iter()
        .find(|e| e.source == owner_runs())
        .expect("owner coverage")
        .state
}

fn twenty_repos() -> Vec<Repo> {
    (0..20)
        .map(|i| repo(&format!("r{i}"), Some("main"), false))
        .collect()
}

fn timed_out() -> TransportError {
    TransportError::TimedOut {
        after: Duration::from_secs(30),
    }
}

fn owner_runs() -> Source {
    Source::Owner {
        owner: "octo".to_owned(),
        kind: SourceKind::Runs,
    }
}

fn repo_runs(name: &str) -> Source {
    Source::Repo {
        repo: repo_id(&format!("octo/{name}")),
        kind: SourceKind::Runs,
    }
}

fn find<'a>(runs: &'a [Run], repo: &str, workflow: &str) -> &'a Run {
    runs.iter()
        .find(|r| {
            r.repo == repo_id(repo)
                && r.workflow == WorkflowId(format!(".github/workflows/{workflow}.yml"))
        })
        .unwrap_or_else(|| panic!("no Run of {workflow} in {repo}: {runs:#?}"))
}

/// "busy": more default-branch Runs than one page. The first page has `ci`
/// (newest failed, an older pass) and a fork's Pull Request run of `lint`
/// from a branch also called `main`; `lint`, `nightly` and `release` need
/// per-workflow lookups, and `release`'s lookup fails.
fn busy_script(path: &str) -> RestResponse {
    let repo = "octo/busy";
    let ci = (1, "ci");
    let lint = (2, "lint");
    let nightly = (3, "nightly");
    match path {
        "repos/octo/busy/actions/runs?branch=main&per_page=100&exclude_pull_requests=true" => {
            ok(runs_page(
                250,
                vec![
                    with(
                        run_node(repo, lint, 310, "failure"),
                        json!({ "event": "pull_request", "head_repository": { "full_name": "stranger/busy" } }),
                    ),
                    run_node(repo, ci, 300, "failure"),
                    run_node(repo, ci, 200, "success"),
                ],
            ))
        }
        "repos/octo/busy/actions/workflows?per_page=100" => ok(json!({
            "total_count": 4,
            "workflows": [
                { "id": 1, "name": "ci" },
                { "id": 2, "name": "lint" },
                { "id": 3, "name": "nightly" },
                { "id": 4, "name": "release" }
            ]
        })),
        "repos/octo/busy/actions/workflows/2/runs?branch=main&per_page=10&exclude_pull_requests=true" => {
            ok(runs_page(40, vec![run_node(repo, lint, 150, "success")]))
        }
        "repos/octo/busy/actions/workflows/3/runs?branch=main&per_page=10&exclude_pull_requests=true" => {
            ok(runs_page(
                5,
                vec![with(
                    run_node(repo, nightly, 120, "failure"),
                    json!({ "run_attempt": 2, "event": "schedule", "triggering_actor": { "login": "me" } }),
                )],
            ))
        }
        "repos/octo/busy/actions/workflows/4/runs?branch=main&per_page=10&exclude_pull_requests=true" => {
            http(502, "Server Error")
        }
        other => panic!("unexpected path {other}"),
    }
}

#[test]
fn a_busy_repo_fills_in_quiet_workflows_and_keeps_only_default_branch_runs() {
    let github = ScriptedGitHub::new(json!([]), busy_script);
    let fetched = fetch(&github, &[repo("busy", Some("main"), false)]).expect("fetch");

    let mut kept: Vec<(String, u64, u32, bool)> = fetched
        .items
        .iter()
        .map(|r| {
            (
                r.workflow.0.clone(),
                r.run_id,
                r.attempt,
                r.on_default_branch,
            )
        })
        .collect();
    kept.sort();
    assert_eq!(
        kept,
        [
            (".github/workflows/ci.yml".to_owned(), 300, 1, true),
            (".github/workflows/lint.yml".to_owned(), 150, 1, true),
            (".github/workflows/nightly.yml".to_owned(), 120, 2, true),
        ],
        "one Run per workflow, the newest on the default branch; the fork's PR run is not one"
    );
    let ci = find(&fetched.items, "octo/busy", "ci");
    assert_eq!(ci.status, RunStatus::Completed);
    assert_eq!(ci.conclusion, Some(RunConclusion::Failure));
    assert_eq!(ci.head_commit, format!("{:040x}", 300));
    assert_eq!(ci.event, "push");
    assert_eq!(ci.triggering_actor.as_deref(), Some("alice"));
    assert_eq!(ci.url, "https://github.com/octo/busy/actions/runs/300");
    assert_eq!(ci.started_at, Some(ts("2026-11-18T11:00:00Z")));
    assert_eq!(ci.updated_at, ts("2026-11-18T12:00:00Z"));
    let nightly = find(&fetched.items, "octo/busy", "nightly");
    assert_eq!(nightly.event, "schedule");
    assert_eq!(nightly.triggering_actor.as_deref(), Some("me"));
    assert_eq!(nightly.actor.as_deref(), Some("alice"));

    // `ci` was on the first page, so it needed no lookup of its own.
    assert!(
        !github
            .paths()
            .iter()
            .any(|p| p.contains("workflows/1/runs")),
        "{:?}",
        github.paths()
    );
    let [busy, owner] = fetched.coverage.as_slice() else {
        panic!("Repo and Owner coverage, got {:?}", fetched.coverage);
    };
    assert_eq!(busy.source, repo_runs("busy"));
    assert!(
        matches!(&busy.state, SourceState::Partial(Incompleteness::PaginationStopped { reason, .. }) if reason.contains("release")),
        "{:?}",
        busy.state
    );
    assert_eq!(
        owner,
        &CoverageEntry {
            source: owner_runs(),
            state: SourceState::Complete
        }
    );
}

#[test]
fn quiet_archived_and_empty_repos_cost_at_most_one_request() {
    let github = ScriptedGitHub::new(json!([]), |path| match path {
        "repos/octo/quiet/actions/runs?branch=main&per_page=100&exclude_pull_requests=true" => {
            ok(runs_page(0, vec![]))
        }
        "repos/octo/few/actions/runs?branch=release%2F1.x&per_page=100&exclude_pull_requests=true" => {
            ok(runs_page(
                1,
                vec![with(
                    run_node("octo/few", (9, "ci"), 7, "timed_out"),
                    json!({ "head_branch": "release/1.x" }),
                )],
            ))
        }
        other => panic!("unexpected path {other}"),
    });
    let fetched = fetch(
        &github,
        &[
            repo("quiet", Some("main"), false),
            repo("few", Some("release/1.x"), false),
            repo("old", Some("main"), true),
            repo("empty", None, false),
        ],
    )
    .expect("fetch");
    assert_eq!(
        github.paths().len(),
        2,
        "one request each for quiet and few; none for the archived or empty Repo: {:?}",
        github.paths()
    );
    let few = find(&fetched.items, "octo/few", "ci");
    assert_eq!(few.conclusion, Some(RunConclusion::TimedOut));
    assert!(few.on_default_branch);
    assert_eq!(
        fetched.coverage,
        ["quiet", "few"]
            .map(|name| CoverageEntry {
                source: repo_runs(name),
                state: SourceState::Complete,
            })
            .into_iter()
            .chain([CoverageEntry {
                source: owner_runs(),
                state: SourceState::Complete,
            }])
            .collect::<Vec<_>>(),
        "explicit Repo completion lets a refresh remove deleted workflow Runs"
    );
}

#[test]
fn a_workflow_is_identified_by_its_whole_path_without_the_ref_suffix() {
    let full_name = "octo/at";
    let github = ScriptedGitHub::new(json!([]), move |_| {
        ok(runs_page(
            3,
            vec![
                with(
                    run_node(full_name, (1, "deploy"), 30, "failure"),
                    json!({ "path": ".github/workflows/deploy@prod.yml@refs/heads/main" }),
                ),
                with(
                    run_node(full_name, (2, "deploy"), 20, "success"),
                    json!({ "path": ".github/workflows/deploy@staging.yml" }),
                ),
                with(
                    run_node(full_name, (3, "ci"), 10, "failure"),
                    json!({ "path": ".github/workflows/ci.yml@refs/heads/main" }),
                ),
            ],
        ))
    });
    let fetched = fetch(&github, &[repo("at", Some("main"), false)]).expect("fetch");
    let mut workflows: Vec<&str> = fetched
        .items
        .iter()
        .map(|r| r.workflow.0.as_str())
        .collect();
    workflows.sort_unstable();
    assert_eq!(
        workflows,
        [
            ".github/workflows/ci.yml",
            ".github/workflows/deploy@prod.yml",
            ".github/workflows/deploy@staging.yml",
        ],
        "an `@` in a file name keeps two workflows apart; only `@refs/...` is dropped"
    );
}

#[test]
fn a_run_from_an_unnamed_source_repo_is_not_a_default_branch_run() {
    let full_name = "octo/orphan";
    let github = ScriptedGitHub::new(json!([]), move |_| {
        ok(runs_page(
            1,
            vec![with(
                run_node(full_name, (1, "ci"), 10, "failure"),
                json!({ "head_repository": null }),
            )],
        ))
    });
    let fetched = fetch(&github, &[repo("orphan", Some("main"), false)]).expect("fetch");
    assert!(
        fetched.items.is_empty(),
        "GitHub didn't say which Repo it ran from, so it isn't known to be this one's: {:#?}",
        fetched.items
    );
}

#[test]
fn a_failed_repo_is_recorded_on_that_repo_and_the_rest_are_still_read() {
    let github = ScriptedGitHub::new(json!([]), |path| {
        if path.starts_with("repos/octo/gone/") {
            http(404, "Not Found")
        } else {
            ok(runs_page(
                1,
                vec![run_node("octo/fine", (1, "ci"), 5, "failure")],
            ))
        }
    });
    let fetched = fetch(
        &github,
        &[
            repo("gone", Some("main"), false),
            repo("fine", Some("main"), false),
        ],
    )
    .expect("fetch");
    assert_eq!(fetched.items.len(), 1);
    let gone = fetched
        .coverage
        .iter()
        .find(|e| e.source == repo_runs("gone"))
        .expect("coverage for gone");
    assert!(
        matches!(&gone.state, SourceState::Failed { error } if error.contains("404")),
        "{:?}",
        gone.state
    );
}

#[test]
fn a_rate_limit_marks_the_owners_runs_partial_with_the_reset_time() {
    let github = ScriptedGitHub::new(json!([]), |_| rate_limited());
    let repos: Vec<Repo> = (0..20)
        .map(|i| repo(&format!("r{i}"), Some("main"), false))
        .collect();
    let fetched = fetch(&github, &repos).expect("fetch");
    assert!(fetched.items.is_empty());
    let reset = jiff::Timestamp::from_second(RESET_EPOCH_SECS).ok();
    let owner = fetched
        .coverage
        .iter()
        .find(|e| e.source == owner_runs())
        .expect("owner coverage");
    assert_eq!(
        owner.state,
        SourceState::Partial(Incompleteness::RateLimited { resets_at: reset })
    );
    assert!(
        github.paths().len() < repos.len(),
        "the remaining Repos are not requested once rate limited ({} requests)",
        github.paths().len()
    );
}

#[test]
fn ordinary_answers_never_stop_the_owners_other_repos() {
    // Twenty Repos, more than the Owner's concurrent requests, so a fetcher
    // that stopped the Owner after any answer (not only a rate limit) would
    // leave some unread. This isolates the rate limit as what stops it.
    let github = ScriptedGitHub::new(json!([]), |path| match path {
        p if p.starts_with("repos/octo/r1/") || p.starts_with("repos/octo/r2/") => {
            http(404, "Not Found")
        }
        p if p.starts_with("repos/octo/r3/") => http(502, "Server Error"),
        _ => ok(runs_page(0, vec![])),
    });
    let repos = twenty_repos();
    let fetched = fetch(&github, &repos).expect("fetch");
    assert_eq!(
        github.paths().len(),
        repos.len(),
        "every Repo read: {:?}",
        github.paths()
    );
    assert_eq!(owner_state(&fetched), &SourceState::Complete);
}

#[test]
fn a_few_timeouts_are_recorded_on_their_repos_and_the_rest_are_still_read() {
    let slow: Vec<String> = (0..runs::MAX_TIMED_OUT_REQUESTS_PER_OWNER)
        .map(|i| format!("repos/octo/r{i}/"))
        .collect();
    let github = ScriptedGitHub::fallible(json!([]), move |path| {
        if slow.iter().any(|s| path.starts_with(s.as_str())) {
            Err(timed_out())
        } else {
            Ok(ok(runs_page(0, vec![])))
        }
    });
    let repos = twenty_repos();
    let fetched = fetch(&github, &repos).expect("fetch");
    assert_eq!(github.paths().len(), repos.len(), "{:?}", github.paths());
    assert_eq!(owner_state(&fetched), &SourceState::Complete);
    let r0 = fetched
        .coverage
        .iter()
        .find(|e| e.source == repo_runs("r0"))
        .expect("coverage for r0");
    assert!(
        matches!(&r0.state, SourceState::Failed { error } if error.contains("did not answer")),
        "{:?}",
        r0.state
    );
}

#[test]
fn repeated_timeouts_stop_the_owner_and_mark_its_runs_partial() {
    let github = ScriptedGitHub::fallible(json!([]), |_| Err(timed_out()));
    let repos = twenty_repos();
    let fetched = fetch(&github, &repos).expect("fetch");
    assert!(
        github.paths().len() < repos.len(),
        "the remaining Repos are not requested once GitHub keeps timing out ({} requests)",
        github.paths().len()
    );
    assert!(
        matches!(
            owner_state(&fetched),
            SourceState::Partial(Incompleteness::PaginationStopped { reason, .. }) if reason.contains("timed out")
        ),
        "{:?}",
        owner_state(&fetched)
    );
}

/// Coverage::record's rule, through the real survey (coverage_seams.rs checks
/// it at seams 1 and 2): each kind of the Owner read completely on its own,
/// but the Repo list came with an error, so every kind is partial.
#[test]
fn a_partly_read_repo_list_makes_every_kind_of_the_owner_partial_in_the_survey() {
    let mut github = ScriptedGitHub::new(json!([repo_node("r", Some("main"), false)]), |_| {
        ok(runs_page(0, vec![]))
    });
    let surveyed = survey(&github, &["octo".to_owned()], now()).expect("survey");
    for kind in SourceKind::WORK_ITEMS {
        assert_eq!(
            surveyed.coverage.get(&Source::Owner {
                owner: "octo".to_owned(),
                kind,
            }),
            Some(&SourceState::Complete),
            "{kind:?} on its own"
        );
    }

    github.repo_list_error = Some(GraphQlError {
        message: "Something went wrong listing a repository".to_owned(),
        kind: None,
    });
    let surveyed = survey(&github, &["octo".to_owned()], now()).expect("survey");
    for kind in SourceKind::WORK_ITEMS {
        assert_eq!(
            surveyed.coverage.get(&Source::Owner {
                owner: "octo".to_owned(),
                kind,
            }),
            Some(&SourceState::Partial(Incompleteness::RepoListIncomplete {
                owner: "octo".to_owned()
            })),
            "{kind:?} under a partly read Repo list"
        );
    }
}

#[test]
fn a_rejected_token_stops_the_survey() {
    let github = ScriptedGitHub::new(json!([]), |_| http(401, "Bad credentials"));
    let result = fetch(&github, &[repo("r", Some("main"), false)]);
    assert!(
        matches!(result, Err(SurveyFailure::NotLoggedIn { .. })),
        "{result:?}"
    );
}

#[test]
fn the_survey_carries_runs_and_their_coverage() {
    let github = ScriptedGitHub::new(json!([repo_node("busy", Some("main"), false)]), busy_script);
    let observation = survey(&github, &["octo".to_owned()], now()).expect("survey");
    assert_eq!(observation.runs.len(), 3);
    assert_eq!(
        observation.coverage.get(&owner_runs()),
        Some(&SourceState::Complete)
    );
    assert!(matches!(
        observation.coverage.get(&repo_runs("busy")),
        Some(SourceState::Partial(_))
    ));
}

// `gh api -i` as the REST transport, against fake `gh` scripts.

/// Serialises fake-`gh` tests (see the note in `github_source.rs`).
static FAKE_GH: Mutex<()> = Mutex::new(());

fn run_fake_gh(body: &str) -> Result<RestResponse, TransportError> {
    let _serial = FAKE_GH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gh");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    GhCli::with_program(path, Duration::from_secs(5))
        .get("repos/octo/r/actions/runs?branch=main&per_page=100")
}

#[test]
fn gh_api_include_output_becomes_a_response_with_status_and_rate_limit() {
    // Echo the arguments back in the body.
    let script = r#"printf 'HTTP/2.0 200 OK\r\nContent-Type: application/json\r\nX-Ratelimit-Remaining: 4982\r\nX-Ratelimit-Reset: 1795136400\r\n\r\n{"args":"%s"}' "$*""#;
    let response = run_fake_gh(script).expect("a response");
    assert_eq!(response.status, 200);
    assert_eq!(response.rate_limit_remaining, Some(4982));
    assert_eq!(
        response.rate_limit_reset,
        jiff::Timestamp::from_second(RESET_EPOCH_SECS).ok()
    );
    assert_eq!(
        response.body,
        Some(
            json!({ "args": "api -i --method GET repos/octo/r/actions/runs?branch=main&per_page=100" })
        )
    );
}

#[test]
fn an_http_error_is_a_response_despite_gh_exiting_non_zero() {
    let script = r#"printf 'HTTP/2.0 404 Not Found\r\nX-Ratelimit-Remaining: 4981\r\n\r\n{"message":"Not Found","status":"404"}gh: Not Found (HTTP 404)\n'
echo 'gh: Not Found (HTTP 404)' >&2
exit 1"#;
    let response = run_fake_gh(script).expect("a response");
    assert_eq!(response.status, 404);
    assert_eq!(
        response.body,
        Some(json!({ "message": "Not Found", "status": "404" }))
    );
}

#[test]
fn gh_failures_without_an_http_response_are_values_with_context() {
    for (script, check) in [
        (
            "echo 'To get started with GitHub CLI, please run:  gh auth login' >&2; exit 4",
            (|e: &TransportError| matches!(e, TransportError::NotLoggedIn { .. }))
                as fn(&TransportError) -> bool,
        ),
        (
            "echo 'dial tcp: lookup api.github.com: no such host' >&2; exit 1",
            |e| matches!(e, TransportError::Failed { detail } if detail.contains("no such host") && detail.contains("exit 1")),
        ),
    ] {
        let error = run_fake_gh(script).expect_err(script);
        assert!(check(&error), "{script}: {error:?}");
    }
}
