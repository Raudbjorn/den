//! Config and paths: Owners come from the config file; missing or malformed
//! config is an error with the file and line, never silently defaulted.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    config::{self, ConfigError, Location},
    domain::{NextStep, RepoId},
    paths::{Paths, PathsError},
};
use std::{collections::HashMap, ffi::OsString, path::Path};

const PATH: &str = "/home/me/.config/den/config.toml";

fn parse(text: &str) -> Result<config::Config, ConfigError> {
    config::parse(Path::new(PATH), text)
}

fn location(error: &ConfigError) -> Option<Location> {
    match error {
        ConfigError::Malformed { location, .. } => *location,
        _ => None,
    }
}

#[test]
fn owners_are_read_in_order() {
    let config =
        parse("owners = [\"Raudbjorn\", \"massif-network\", \"example-org\"]\n").expect("valid");
    assert_eq!(
        config.owners,
        ["Raudbjorn", "massif-network", "example-org"]
    );
    assert_eq!(config.main_clone_root, None);
    assert!(config.main_clones.is_empty());
}

#[test]
fn later_fields_are_accepted() {
    let config = parse(
        "owners = [\"a\"]\nmain_clone_root = \"/home/me/dev\"\n\n[main_clones]\n\"a/b\" = \"/x/b\"\n",
    )
    .expect("valid");
    assert_eq!(
        config.main_clone_root.as_deref(),
        Some(Path::new("/home/me/dev"))
    );
    assert_eq!(
        config.main_clones.get("a/b").map(|p| p.as_path()),
        Some(Path::new("/x/b"))
    );
}

#[test]
fn malformed_config_names_the_file_and_line() {
    let cases = [
        ("syntax", "# watched\nowners = [\"a\",\n", 2),
        ("wrong type", "\nowners = [\"a\", 3]\n", 2),
        ("typo'd field", "owners = [\"a\"]\n\nonwers = [\"b\"]\n", 3),
        (
            "not a login",
            "owners = [\n  \"good\",\n  \"has space\",\n]\n",
            3,
        ),
        ("duplicate", "owners = [\"Octo\",\n \"octo\"]\n", 2),
        ("empty", "\n\nowners = []\n", 3),
        (
            "relative clone root",
            "owners = [\"a\"]\nmain_clone_root = \"dev\"\n",
            2,
        ),
        (
            "relative Main Clone",
            "owners = [\"a\"]\n\n[main_clones]\n\"a/b\" = \"~/dev/b\"\n",
            4,
        ),
    ];
    for (name, text, line) in cases {
        let error = parse(text).expect_err(name);
        let message = error.to_string();
        assert!(message.starts_with(PATH), "{name}: {message}");
        assert_eq!(
            location(&error).map(|l| l.line),
            Some(line),
            "{name}: {message}"
        );
        assert!(
            message.contains(&format!("line {line}")),
            "{name}: {message}"
        );
    }
}

#[test]
fn a_relative_main_clone_path_is_refused_with_why() {
    let error =
        parse("owners = [\"a\"]\n\n[main_clones]\n\"a/b\" = \"b\"\n").expect_err("relative");
    let message = error.to_string();
    assert!(
        message.ends_with(
            "the Main Clone of a/b must be an absolute path, not \"b\" (den doesn't expand ~ or resolve paths against the folder it runs in)"
        ),
        "{message}"
    );
}

#[test]
fn missing_owners_is_malformed_and_names_the_file() {
    let error = parse("main_clone_root = \"/x\"\n").expect_err("no owners");
    assert!(matches!(error, ConfigError::Malformed { .. }), "{error:?}");
    let message = error.to_string();
    assert!(
        message.starts_with(PATH) && message.contains("owners"),
        "{message}"
    );
}

#[test]
fn a_missing_file_is_an_error_naming_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("den/config.toml");
    let error = config::load(&path).expect_err("missing");
    assert_eq!(error, ConfigError::Missing { path: path.clone() });
    assert!(error.to_string().contains(&path.display().to_string()));
}

#[test]
fn a_file_on_disk_is_loaded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "owners = [\"octo\"]\n").expect("write");
    assert_eq!(config::load(&path).expect("loads").owners, ["octo"]);
}

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
        .collect();
    move |name| map.get(name).cloned()
}

#[test]
fn paths_follow_xdg_and_fall_back_to_home() {
    let xdg = Paths::from_env(env(&[
        ("HOME", "/home/me"),
        ("XDG_CONFIG_HOME", "/cfg"),
        ("XDG_STATE_HOME", "/state"),
    ]))
    .expect("paths");
    assert_eq!(xdg.config_file, Path::new("/cfg/den/config.toml"));
    assert_eq!(xdg.log_file(), Path::new("/state/den/den.log"));

    let home = Paths::from_env(env(&[("HOME", "/home/me"), ("XDG_STATE_HOME", "relative")]))
        .expect("paths");
    assert_eq!(
        home.config_file,
        Path::new("/home/me/.config/den/config.toml")
    );
    assert_eq!(
        home.state_dir,
        Path::new("/home/me/.local/state/den"),
        "relative XDG ignored"
    );

    assert!(matches!(
        Paths::from_env(env(&[])),
        Err(PathsError::NoHome { .. })
    ));
}

/// The user's Claude directory (#41) is Claude Code's: `$CLAUDE_CONFIG_DIR`
/// when it is absolute, else `~/.claude`.
#[test]
fn the_claude_directory_follows_claude_config_dir_then_home() {
    let set = Paths::from_env(env(&[
        ("HOME", "/home/me"),
        ("CLAUDE_CONFIG_DIR", "/home/me/.claude-work"),
    ]))
    .expect("paths");
    assert_eq!(
        set.claude_dir.as_deref(),
        Some(Path::new("/home/me/.claude-work"))
    );
    let relative = Paths::from_env(env(&[("HOME", "/home/me"), ("CLAUDE_CONFIG_DIR", "work")]))
        .expect("paths");
    assert_eq!(
        relative.claude_dir.as_deref(),
        Some(Path::new("/home/me/.claude"))
    );
    let no_home = Paths::from_env(env(&[
        ("XDG_CONFIG_HOME", "/cfg"),
        ("XDG_STATE_HOME", "/state"),
    ]))
    .expect("paths");
    assert_eq!(no_home.claude_dir, None);
}

// Linear Workflows (#32).

const FULL_WORKFLOW: &str = r##"owners = ["acme", "octo"]

[linear.acme]
workspace = "Acme"
api_key_env = "DEN_LINEAR_KEY"
start_teams = ["TEAM", "OPS"]
include_backlog = true
active_cycle_only = true
new_work_repos = ["api", "acme/web"]
held_repos = ["app"]
landing_order = ["api", "web"]
reviewers = ["alice", "acme/backend"]
slack_channel = "#team-dev"
in_progress_state = "In Progress"
release_state = "Ready for Release"
parked_states = ["Paused", "Blocked"]
exact_head_approval = true
comment_reviews_need_rereview = false
pending_requests_hold_merge = true
"##;

#[test]
fn a_linear_workflow_reads_every_setting() {
    let config = parse(FULL_WORKFLOW).expect("valid");
    assert_eq!(config.linear.len(), 1);
    let workflow = config.linear_workflow("ACME").expect("acme's");
    assert_eq!(workflow.owner, "acme");
    assert_eq!(workflow.workspace, "acme", "lowercased");
    assert_eq!(workflow.api_key_env, "DEN_LINEAR_KEY");
    assert_eq!(workflow.api_key_env_at.line, 5);
    assert_eq!(workflow.start_teams, ["TEAM", "OPS"]);
    assert!(workflow.include_backlog && workflow.active_cycle_only);
    let names =
        |repos: &[den::domain::RepoId]| repos.iter().map(|r| r.to_string()).collect::<Vec<_>>();
    assert_eq!(names(&workflow.new_work_repos), ["acme/api", "acme/web"]);
    assert_eq!(names(&workflow.held_repos), ["acme/app"]);
    assert_eq!(names(&workflow.landing_order), ["acme/api", "acme/web"]);
    assert_eq!(
        workflow.reviewers,
        [
            config::Reviewer::User("alice".to_owned()),
            config::Reviewer::Team(den::domain::TeamRef {
                org: "acme".to_owned(),
                slug: "backend".to_owned(),
            }),
        ]
    );
    assert_eq!(workflow.slack_channel.as_deref(), Some("#team-dev"));
    assert_eq!(workflow.in_progress_state.as_deref(), Some("In Progress"));
    assert_eq!(workflow.release_state.as_deref(), Some("Ready for Release"));
    assert_eq!(workflow.parked_states, ["Paused", "Blocked"]);
    // Explicit strict settings still override the GitHub-aligned defaults.
    assert!(workflow.exact_head_approval);
    assert!(!workflow.comment_reviews_need_rereview);
    assert!(workflow.pending_requests_hold_merge);
    assert!(config.linear_workflow("octo").is_none());
}

/// The review switches default to GitHub's own rules (ADR-0009, #73):
/// exact-head approval and pending requests holding the merge off, comment
/// reviews needing re-review on. Everything else is empty or off, and no
/// organization's values ship.
#[test]
fn a_minimal_linear_workflow_has_the_configs_defaults() {
    let config =
        parse("owners = [\"acme\"]\n[linear.acme]\nworkspace = \"acme\"\napi_key_env = \"K\"\n")
            .expect("valid");
    let workflow = &config.linear[0];
    assert!(!workflow.exact_head_approval);
    assert!(workflow.comment_reviews_need_rereview);
    assert!(!workflow.pending_requests_hold_merge);
    assert!(!workflow.include_backlog && !workflow.active_cycle_only);
    assert!(workflow.start_teams.is_empty() && workflow.new_work_repos.is_empty());
    assert!(workflow.held_repos.is_empty() && workflow.landing_order.is_empty());
    assert!(workflow.reviewers.is_empty() && workflow.slack_channel.is_none());
    assert!(workflow.in_progress_state.is_none() && workflow.release_state.is_none());
    assert!(
        workflow.parked_states.is_empty(),
        "nothing is parked by default"
    );
}

#[test]
fn owners_without_a_linear_workflow_get_the_empty_policy() {
    let config = parse(FULL_WORKFLOW).expect("valid");
    let policies = config.policies();
    assert_eq!(
        policies.for_owner("octo"),
        &den::domain::LinearWorkflowPolicy::default()
    );
    let github_only = parse("owners = [\"acme\"]\n").expect("valid");
    assert!(github_only.linear.is_empty());
    assert_eq!(github_only.policies(), den::domain::Policies::default());
}

/// #36: a Linear Workflow's review settings and Held Repos reach the decision
/// engine as the Owner's policy, the switches as configured; so do its start
/// teams and their two switches (#33), its release state (#37) and its
/// landing order (#40). `Config::policies` maps every field.
#[test]
fn a_linear_workflows_review_settings_become_its_owners_policy() {
    use den::domain::{
        LinearWorkflowPolicy, RepoId, ReviewPolicy, StartPolicy, TeamRef, facts::Reviewer,
    };
    let config = parse(FULL_WORKFLOW).expect("valid");
    assert_eq!(
        config.policies().for_owner("ACME"),
        &LinearWorkflowPolicy {
            start: StartPolicy {
                teams: vec!["TEAM".to_owned(), "OPS".to_owned()],
                include_backlog: true,
                active_cycle_only: true,
            },
            review: Some(ReviewPolicy {
                exact_head_approval: true,
                comment_reviews_need_rereview: false,
                pending_requests_hold_merge: true,
                reviewers: vec![
                    Reviewer::User("alice".to_owned()),
                    Reviewer::Team(TeamRef {
                        org: "acme".to_owned(),
                        slug: "backend".to_owned(),
                    }),
                ],
            }),
            held_repos: vec![RepoId::new("acme", "app")],
            release_state: Some("Ready for Release".to_owned()),
            landing_order: vec![RepoId::new("acme", "api"), RepoId::new("acme", "web")],
            parked_states: vec!["Paused".to_owned(), "Blocked".to_owned()],
        }
    );
    // The config's defaults (ADR-0009): GitHub's rules, comments re-reviewed.
    let minimal =
        parse("owners = [\"acme\"]\n[linear.acme]\nworkspace = \"acme\"\napi_key_env = \"K\"\n")
            .expect("valid");
    let policies = minimal.policies();
    let review = policies
        .for_owner("acme")
        .review
        .as_ref()
        .expect("review rules");
    assert!(!review.exact_head_approval && review.comment_reviews_need_rereview);
    assert!(!review.pending_requests_hold_merge);
    assert!(review.reviewers.is_empty());
    // The empty policy keeps every switch off, offers no Start and holds
    // nothing.
    let empty = LinearWorkflowPolicy::default();
    assert_eq!(empty.start, StartPolicy::NONE);
    assert_eq!(empty.review, None);
    assert!(empty.held_repos.is_empty());
    assert_eq!(empty.release_state, None);
    assert!(empty.parked_states.is_empty());
}

#[test]
fn linear_workflow_errors_name_the_file_and_line() {
    let base = "owners = [\"acme\", \"octo\"]\n";
    let block = |body: &str| {
        format!("{base}\n[linear.acme]\nworkspace = \"acme\"\napi_key_env = \"K\"\n{body}")
    };
    let cases: Vec<(&str, String, usize, &str)> = vec![
        (
            "unwatched Owner",
            format!("{base}\n[linear.nobody]\nworkspace = \"acme\"\napi_key_env = \"K\"\n"),
            3,
            "not in `owners`",
        ),
        (
            "another Owner's Repo",
            block("held_repos = [\"api\",\n  \"octo/api\"]\n"),
            7,
            "not one of acme's Repos",
        ),
        (
            "not a Repo name",
            block("new_work_repos = [\"a b\"]\n"),
            6,
            "not a Repo name",
        ),
        (
            "duplicate Repo",
            block("landing_order = [\"api\", \"API\"]\n"),
            6,
            "listed twice",
        ),
        (
            // The same Repo written with and without its Owner.
            "duplicate Repo, two spellings",
            block("new_work_repos = [\"api\",\n  \"Acme/API\"]\n"),
            7,
            "listed twice",
        ),
        (
            "duplicate reviewer team, two casings",
            block("reviewers = [\"acme/backend\", \"ACME/Backend\"]\n"),
            6,
            "listed twice",
        ),
        (
            "bad workspace",
            format!(
                "{base}\n[linear.acme]\nworkspace = \"linear.app/acme\"\napi_key_env = \"K\"\n"
            ),
            4,
            "workspace",
        ),
        (
            "a key, not a name",
            format!("{base}\n[linear.acme]\nworkspace = \"acme\"\napi_key_env = \"lin_api_x y\"\n"),
            5,
            "environment variable name",
        ),
        (
            "bad team key",
            block("start_teams = [\"TEAM-1\"]\n"),
            6,
            "team key",
        ),
        (
            "bad reviewer",
            block("reviewers = [\"-x\"]\n"),
            6,
            "reviewers",
        ),
        (
            "blank state",
            block("release_state = \"  \"\n"),
            6,
            "release_state",
        ),
        (
            "blank parked state",
            block("parked_states = [\"Paused\", \" \"]\n"),
            6,
            "parked_states",
        ),
        (
            "a parked state listed twice",
            block("parked_states = [\"Paused\", \"PAUSED\"]\n"),
            6,
            "listed twice in `parked_states`",
        ),
        // A type error is TOML's own: it names the line and column of the
        // value, not the key, as for every list setting.
        (
            "parked_states not a list",
            block("parked_states = \"Paused\"\n"),
            6,
            "expected a sequence",
        ),
        (
            "unknown setting",
            block("held_repo = [\"app\"]\n"),
            6,
            "held_repo",
        ),
        (
            "missing api_key_env",
            format!("{base}\n[linear.acme]\nworkspace = \"acme\"\n"),
            3,
            "api_key_env",
        ),
        (
            "a workspace bound twice",
            format!(
                "{base}\n[linear.acme]\nworkspace = \"acme\"\napi_key_env = \"K\"\n\n[linear.octo]\nworkspace = \"ACME\"\napi_key_env = \"K\"\n"
            ),
            7,
            "bound to both",
        ),
    ];
    for (name, text, line, says) in cases {
        let error = parse(&text).expect_err(name);
        let message = error.to_string();
        assert!(message.starts_with(PATH), "{name}: {message}");
        assert!(message.contains(says), "{name}: {message}");
        assert_eq!(
            location(&error).map(|l| l.line),
            Some(line),
            "{name}: {message}\n{text}"
        );
    }
}

#[test]
fn a_missing_key_variable_is_an_error_naming_it_and_its_line() {
    let config = parse(FULL_WORKFLOW).expect("valid");
    let path = Path::new(PATH);
    assert_eq!(
        config.check_linear_keys(path, env(&[("DEN_LINEAR_KEY", "lin_api_x")])),
        Ok(())
    );
    for unset in [env(&[]), env(&[("DEN_LINEAR_KEY", "")])] {
        let error = config.check_linear_keys(path, unset).expect_err("unset");
        let message = error.to_string();
        assert!(message.starts_with(&format!("{PATH}, line 5")), "{message}");
        assert!(message.contains("DEN_LINEAR_KEY"), "{message}");
        assert!(message.contains("acme"), "{message}");
    }
    // Owners without a Linear Workflow need no key.
    let github_only = parse("owners = [\"acme\"]\n").expect("valid");
    assert_eq!(github_only.check_linear_keys(path, env(&[])), Ok(()));
}

/// Review on #50: the config's `reviewers` and the Request review sheet share
/// one grammar (`handoff::reviewers::parse`), so a reviewer the config takes
/// is one the sheet takes, and the other way round.
#[test]
fn config_reviewers_and_the_sheets_reviewers_accept_the_same_words() {
    let base = "owners = [\"acme\"]\n[linear.acme]\nworkspace = \"acme\"\napi_key_env = \"K\"\n";
    for word in [
        "alice",
        "acme/backend",
        "acme/web.team_2",
        "-x",
        "x-",
        "a b",
        "acme/",
        "/backend",
        "a/b/c",
        "acme/back end",
    ] {
        let config = parse(&format!("{base}reviewers = [{word:?}]\n"));
        let sheet = den::handoff::reviewers::parse(word);
        assert_eq!(
            config.as_ref().ok().map(|c| c.linear[0].reviewers.clone()),
            sheet.clone().map(|r| vec![r]),
            "{word:?}: config {config:?}, sheet {sheet:?}"
        );
    }
}

#[test]
fn repos_can_skip_next_step_kinds() {
    let config = parse(
        "owners = [\"acme\"]\n\n[repos.\"acme/hobby\"]\nskip = [\"Fix CI\", \"Triage\", \"Prune or revive\"]\n",
    )
    .expect("valid");
    let hobby = RepoId::new("acme", "hobby");
    assert_eq!(
        config
            .skips
            .for_repo(&hobby)
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [NextStep::FixCi, NextStep::Triage, NextStep::PruneOrRevive]
    );
    assert!(
        config
            .skips
            .for_repo(&RepoId::new("acme", "other"))
            .is_empty()
    );
    assert!(
        config
            .policies()
            .skips()
            .for_repo(&hobby)
            .contains(&NextStep::FixCi)
    );
}

#[test]
fn repo_keys_compare_without_case() {
    let config = parse("owners = [\"acme\"]\n\n[repos.\"ACME/Hobby\"]\nskip = [\"Triage\"]\n")
        .expect("valid");
    assert!(
        config
            .skips
            .for_repo(&RepoId::new("acme", "hobby"))
            .contains(&NextStep::Triage)
    );
}

#[test]
fn a_repo_outside_the_watched_owners_may_skip_too() {
    let config = parse("owners = [\"acme\"]\n\n[repos.\"elsewhere/app\"]\nskip = [\"Merge\"]\n")
        .expect("valid");
    assert!(
        config
            .skips
            .for_repo(&RepoId::new("elsewhere", "app"))
            .contains(&NextStep::Merge)
    );
}

#[test]
fn no_repos_table_skips_nothing() {
    let config = parse("owners = [\"acme\"]\n").expect("valid");
    assert!(
        config
            .skips
            .for_repo(&RepoId::new("acme", "app"))
            .is_empty()
    );
}

#[test]
fn bad_skips_name_the_line() {
    let cases = [
        (
            "wrong case",
            "owners = [\"a\"]\n[repos.\"a/b\"]\nskip = [\"fix ci\"]\n",
            3,
            "\"fix ci\" is not a Next Step: use one of \"Finish draft\"",
        ),
        (
            "a Linear Issue's kind",
            "owners = [\"a\"]\n[repos.\"a/b\"]\nskip = [\"Land\"]\n",
            3,
            "Land belongs to Linear Issues, not to a Repo",
        ),
        (
            "not owner/repo",
            "owners = [\"a\"]\n[repos.\"just-a-name\"]\nskip = []\n",
            2,
            "\"just-a-name\" is not OWNER/REPO",
        ),
        (
            "typo'd field",
            "owners = [\"a\"]\n[repos.\"a/b\"]\nskips = [\"Triage\"]\n",
            3,
            "unknown field",
        ),
        (
            // BTreeMap order puts "a/B" before "a/b", so "a/b" (line 2) is the repeat.
            "the same Repo twice",
            "owners = [\"a\"]\n[repos.\"a/b\"]\nskip = []\n[repos.\"a/B\"]\nskip = []\n",
            2,
            "a/b is configured twice",
        ),
    ];
    for (name, text, line, message) in cases {
        let error = parse(text).expect_err(name);
        assert_eq!(
            location(&error).map(|l| l.line),
            Some(line),
            "{name}: {error}"
        );
        assert!(error.to_string().contains(message), "{name}: {error}");
    }
}

// Merge method per Repo (#72, spec 0004 story 23).

#[test]
fn a_repo_can_name_its_merge_method() {
    use den::domain::MergeMethod;
    let config = parse(
        "owners = [\"acme\"]\n\n[repos.\"acme/app\"]\nmerge_method = \"squash\"\n\n[repos.\"acme/api\"]\nskip = [\"Triage\"]\nmerge_method = \"rebase\"\n\n[repos.\"acme/web\"]\nskip = [\"Fix CI\"]\n",
    )
    .expect("valid");
    let method = |name: &str| config.merge_methods.for_repo(&RepoId::new("acme", name));
    assert_eq!(method("app"), Some(MergeMethod::Squash));
    assert_eq!(
        method("API"),
        Some(MergeMethod::Rebase),
        "Repo names compare without case"
    );
    assert_eq!(
        method("web"),
        None,
        "a Repo table without merge_method names none"
    );
    assert_eq!(method("other"), None);
    assert_eq!(MergeMethod::Squash.flag(), "--squash");
    assert_eq!(MergeMethod::Merge.flag(), "--merge");
    assert_eq!(MergeMethod::Rebase.flag(), "--rebase");
}

#[test]
fn a_bad_merge_method_names_the_line() {
    for (name, text, line) in [
        (
            "not a method",
            "owners = [\"a\"]\n[repos.\"a/b\"]\nmerge_method = \"fast-forward\"\n",
            3,
        ),
        (
            "methods are lowercase, as gh spells them",
            "owners = [\"a\"]\n[repos.\"a/b\"]\nskip = []\nmerge_method = \"Squash\"\n",
            4,
        ),
    ] {
        let error = parse(text).expect_err(name);
        assert_eq!(
            location(&error).map(|l| l.line),
            Some(line),
            "{name}: {error}"
        );
        let message = error.to_string();
        assert!(
            message.contains("merge_method")
                && message.contains("\"squash\", \"merge\" or \"rebase\""),
            "{name}: {message}"
        );
    }
}

/// #75: `legacy_marker_prefixes` is a top-level list of marker prefixes,
/// handed to the decision engine through the policies.
#[test]
fn legacy_marker_prefixes_are_read_in_order() {
    let config = parse("owners = [\"acme\"]\nlegacy_marker_prefixes = [\"acme-bot\", \"tool2\"]\n")
        .expect("valid");
    assert_eq!(config.legacy_marker_prefixes, ["acme-bot", "tool2"]);
    assert_eq!(
        config.policies().legacy_marker_prefixes(),
        ["acme-bot", "tool2"]
    );
    let none = parse("owners = [\"acme\"]\n").expect("valid");
    assert!(none.legacy_marker_prefixes.is_empty());
}

#[test]
fn a_bad_legacy_marker_prefix_is_an_error_naming_its_line() {
    for (prefix, says) in [
        (
            "Acme",
            "is not a marker prefix: use lowercase letters, digits and -",
        ),
        ("acme_bot", "is not a marker prefix"),
        ("", "is not a marker prefix"),
        ("den", "is den's own marker prefix"),
    ] {
        let text =
            format!("owners = [\"acme\"]\n\nlegacy_marker_prefixes = [\"ok\", \"{prefix}\"]\n");
        let error = parse(&text).expect_err(prefix);
        assert!(error.to_string().contains(says), "{prefix:?}: {error}");
        assert_eq!(
            location(&error).map(|l| l.line),
            Some(3),
            "{prefix:?}: {error}"
        );
    }
    let twice = parse("owners = [\"acme\"]\nlegacy_marker_prefixes = [\"a\", \"a\"]\n")
        .expect_err("listed twice");
    assert!(twice.to_string().contains("listed twice"), "{twice}");
}
