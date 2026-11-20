//! Seam 2: forks and Upstream (#13). A fork is its own target until `u` in the
//! sidebar switches it to its Upstream (saved, and read back at the next start);
//! `u` on the launch sheet switches one Job only; `u` elsewhere changes
//! nothing. A PR whose head lives in another Repo is fetched by its PR ref from
//! the target Repo's remote.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Event, Key,
        view_model::{LaunchSheet, LaunchStatus, Overlay, SidebarRow, Tone, ViewModel},
    },
    config::Config,
    domain::{Observation, UpstreamTargets},
    handoff::{BaseRef, BranchStart, LaunchPlan},
    ordering::Scope,
    ports::{WorktreeAdd, WorktreeStart},
    upstreams::UpstreamStoreError,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use support::{
    build::{ME, ObservationBuilder, branch, issue, pr, repo_id, run},
    fakes::{FakeCache, FakeUpstreams, WorkspaceCall},
    harness::Harness,
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/app";
/// The default head commit of PR 7 (`pr()` uses the number in hex).
const HEAD_7: &str = "0000000000000000000000000000000000000007";
const FORK_MAIN: &str = "1111111111111111111111111111111111111111";
const UPSTREAM_MAIN: &str = "2222222222222222222222222222222222222222";
const PR_7: &str = "octo/app#7";
const ISSUE_9: &str = "octo/app#9";
const FORK: &str = "octo/app";
const UPSTREAM: &str = "upstream-org/app";

fn config() -> Config {
    Config {
        owners: vec!["octo".to_owned()],
        main_clone_root: Some(PathBuf::from(ROOT)),
        main_clones: BTreeMap::new(),
    }
}

/// octo/app, a fork of upstream-org/app, with PR #7 (Address review, mine),
/// Issue #9 (Implement, mine), a plain Repo octo/lib, and a Branch in the
/// fork compared against the fork's own default branch.
fn forked() -> Observation {
    ObservationBuilder::new()
        .owner("octo")
        .fork(FORK, Some(UPSTREAM))
        .repo("octo/lib")
        .pull_request(pr(FORK, 7).author(ME).changes_requested())
        .issue(issue(FORK, 9).title("Add a login page").assignee(ME))
        .branch(
            branch(FORK, "my-fix")
                .compared_against(FORK, "main")
                .author(ME),
        )
        .build()
}

/// den starts over `upstreams` (the state directory), and the survey answers.
fn started_with(upstreams: &Arc<FakeUpstreams>, observation: Observation) -> Harness {
    let mut harness = Harness::with_stores(Ok(config()), FakeCache::default(), upstreams.clone());
    harness.answer(Ok(observation));
    harness.start();
    harness
}

/// A Main Clone of the fork, with `origin` → the fork and `upstream` → its Upstream.
fn fork_clone(harness: &Harness) {
    harness.workspace.add_clone(
        CLONE,
        &[
            ("origin", "git@github.com:octo/app.git"),
            ("upstream", "https://github.com/upstream-org/app"),
        ],
    );
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-7", HEAD_7);
    // Where each target's default branch is, for new branches (Issues, Runs).
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "main", FORK_MAIN);
    harness
        .workspace
        .set_remote_head(CLONE, "upstream", "main", UPSTREAM_MAIN);
}

/// Selects the row `reference` (as the list shows it) and presses `c`.
fn open_sheet(harness: &mut Harness, reference: &str) {
    let rows: Vec<String> = harness
        .view
        .list
        .rows
        .iter()
        .map(|r| r.id.to_string())
        .collect();
    let index = rows
        .iter()
        .position(|r| r == reference)
        .unwrap_or_else(|| panic!("no row {reference} in {rows:?}"));
    while harness.view.list.selected != Some(index) {
        let before = harness.view.list.selected;
        let key = if before.is_some_and(|s| s > index) {
            Key::Up
        } else {
            Key::Down
        };
        press(harness, key);
        assert_ne!(before, harness.view.list.selected, "selection stuck");
    }
    press(harness, Key::Char('c'));
}

fn press(harness: &mut Harness, key: Key) {
    harness.settle(Event::Key(key));
}

/// Moves the sidebar cursor onto `scope`'s row (focusing the sidebar first).
fn sidebar_cursor_on(harness: &mut Harness, scope: &Scope) {
    if !harness.view.sidebar.focused {
        press(harness, Key::Tab);
    }
    let target = harness
        .view
        .sidebar
        .rows
        .iter()
        .position(|r| &r.scope == scope)
        .unwrap_or_else(|| panic!("no row for {scope:?}"));
    for _ in 0..harness.view.sidebar.rows.len() {
        match harness.view.sidebar.cursor {
            Some(i) if i == target => return,
            Some(i) if i > target => press(harness, Key::Up),
            _ => press(harness, Key::Down),
        }
    }
    assert_eq!(harness.view.sidebar.cursor, Some(target));
}

fn repo_scope(full_name: &str) -> Scope {
    Scope::Repo(repo_id(full_name))
}

fn row<'a>(view: &'a ViewModel, scope: &Scope) -> &'a SidebarRow {
    view.sidebar
        .rows
        .iter()
        .find(|r| &r.scope == scope)
        .unwrap_or_else(|| panic!("no sidebar row for {scope:?}"))
}

fn sheet(harness: &Harness) -> &LaunchSheet {
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => sheet,
        other => panic!("no launch sheet: {other:?}"),
    }
}

fn target(sheet: &LaunchSheet) -> (String, Option<String>, Tone) {
    let field = sheet
        .fields
        .iter()
        .find(|f| f.label == "TARGET")
        .unwrap_or_else(|| panic!("no TARGET: {sheet:#?}"));
    (field.value.clone(), field.note.clone(), field.tone)
}

fn messages(sheet: &LaunchSheet) -> String {
    sheet
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn launch_plans(harness: &Harness) -> Vec<LaunchPlan> {
    harness
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Launch { plan, .. } => Some((**plan).clone()),
            _ => None,
        })
        .collect()
}

fn stores(effects: &[Effect]) -> Vec<UpstreamTargets> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::StoreUpstreams { upstreams } => Some(upstreams.clone()),
            _ => None,
        })
        .collect()
}

fn targets(forks: &[&str]) -> UpstreamTargets {
    UpstreamTargets::new(forks.iter().map(|f| repo_id(f)))
}

fn step_of(harness: &Harness, reference: &str) -> String {
    harness
        .rows()
        .into_iter()
        .find(|(_, r, _)| r == reference)
        .unwrap_or_else(|| panic!("no row {reference}: {:?}", harness.rows()))
        .2
}

#[test]
fn on_a_fork_the_default_target_is_the_fork_and_u_in_the_sidebar_flips_it_and_the_flip_survives_a_restart()
 {
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, forked());
    fork_clone(&harness);

    // By default the fork is its own target. (Decision b: an Issue's sheet
    // follows the fork's setting; a PR's would start on the fork anyway.)
    assert_eq!(row(&harness.view, &repo_scope(FORK)).upstream, None);
    open_sheet(&mut harness, ISSUE_9);
    let (value, note, tone) = target(sheet(&harness));
    assert_eq!(value, FORK);
    assert_eq!(
        note.as_deref(),
        Some("fork: its own Repo, not its Upstream")
    );
    assert_eq!(tone, Tone::Plain);
    press(&mut harness, Key::Esc);

    // `u` on the fork's row: its target is now its Upstream, saved.
    sidebar_cursor_on(&mut harness, &repo_scope(FORK));
    let effects = harness.key(Key::Char('u'));
    assert_eq!(stores(&effects), [targets(&[FORK])]);
    for effect in &effects {
        harness.perform(effect);
    }
    assert_eq!(store.stored(), [targets(&[FORK])]);
    assert_eq!(
        row(&harness.view, &repo_scope(FORK)).upstream.as_deref(),
        Some(UPSTREAM)
    );

    // A restart over the same state directory: still Upstream.
    let mut restarted = started_with(&store, forked());
    fork_clone(&restarted);
    assert_eq!(
        restarted.github.requests()[0].upstreams,
        targets(&[FORK]),
        "the survey compares the fork's Branches against its Upstream"
    );
    assert_eq!(
        row(&restarted.view, &repo_scope(FORK)).upstream.as_deref(),
        Some(UPSTREAM)
    );
    open_sheet(&mut restarted, ISSUE_9);
    let (value, note, tone) = target(sheet(&restarted));
    assert_eq!(value, format!("upstream → {UPSTREAM}"));
    assert_eq!(tone, Tone::Upstream);
    assert!(note.unwrap().contains("as set for the fork"));
    press(&mut restarted, Key::Esc);

    // `u` again: back to the fork itself, saved too.
    sidebar_cursor_on(&mut restarted, &repo_scope(FORK));
    let effects = restarted.key(Key::Char('u'));
    assert_eq!(stores(&effects), [UpstreamTargets::default()]);
    assert_eq!(row(&restarted.view, &repo_scope(FORK)).upstream, None);
}

#[test]
fn the_per_job_override_on_the_sheet_changes_only_that_jobs_plan() {
    // The fork's setting is Upstream. An Issue's sheet follows it (decision
    // b); before decision b this test used PR #7, whose sheet no longer does.
    let store = Arc::new(FakeUpstreams::holding(targets(&[FORK])));
    let mut harness = started_with(&store, forked());
    fork_clone(&harness);

    open_sheet(&mut harness, ISSUE_9);
    let upstream_sheet = sheet(&harness).clone();
    assert_eq!(
        upstream_sheet.status,
        LaunchStatus::Ready,
        "{}",
        messages(&upstream_sheet)
    );
    assert_eq!(target(&upstream_sheet).0, format!("upstream → {UPSTREAM}"));
    assert!(
        harness
            .view
            .footer
            .hints
            .iter()
            .any(|h| h.key == "u" && h.label == "the fork")
    );

    // `u` on the sheet: this Job targets the fork, and launches there.
    press(&mut harness, Key::Char('u'));
    let fork_sheet = sheet(&harness).clone();
    assert_eq!(
        fork_sheet.status,
        LaunchStatus::Ready,
        "{}",
        messages(&fork_sheet)
    );
    let (value, note, tone) = target(&fork_sheet);
    assert_eq!(value, FORK);
    assert_eq!(
        note.as_deref(),
        Some("fork: its own Repo, not its Upstream, for this Job only")
    );
    assert_eq!(tone, Tone::Plain);
    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].target.repo, repo_id(FORK));
    assert_eq!(plans[0].remote.name, "origin");
    assert_eq!(plans[0].base.commit, FORK_MAIN);

    // The fork's own setting never changed: nothing stored, the sidebar still
    // shows the Upstream, and the next sheet starts from the Upstream again.
    assert!(stores(&harness.effects).is_empty());
    assert!(store.stored().is_empty());
    assert_eq!(
        row(&harness.view, &repo_scope(FORK)).upstream.as_deref(),
        Some(UPSTREAM)
    );
    press(&mut harness, Key::Esc);
    open_sheet(&mut harness, ISSUE_9);
    // The Job just launched is live, so the sheet asks about it (#14) rather
    // than showing a TARGET; what it inspected for is the Upstream again.
    assert_eq!(sheet(&harness).status, LaunchStatus::LiveJob);
    assert_eq!(last_inspected_target(&harness), repo_id(UPSTREAM));
}

/// The target of the last workspace inspection the App asked for.
fn last_inspected_target(harness: &Harness) -> den::domain::RepoId {
    harness
        .effects
        .iter()
        .rev()
        .find_map(|e| match e {
            Effect::InspectLaunch { request, .. } => Some(request.target.clone()),
            _ => None,
        })
        .expect("an inspection")
}

#[test]
fn a_pr_sheet_starts_on_the_prs_own_repo_even_when_the_fork_targets_its_upstream() {
    // Decision b: a fork's saved Upstream choice does not apply to PR sheets.
    let store = Arc::new(FakeUpstreams::holding(targets(&[FORK])));
    let mut harness = started_with(&store, forked());
    fork_clone(&harness);

    open_sheet(&mut harness, PR_7);
    let pr_sheet = sheet(&harness).clone();
    assert_eq!(
        pr_sheet.status,
        LaunchStatus::Ready,
        "{}",
        messages(&pr_sheet)
    );
    let (value, note, tone) = target(&pr_sheet);
    assert_eq!(value, FORK);
    assert_eq!(
        note.as_deref(),
        Some(
            "fork: its own Repo, not its Upstream; the fork targets its Upstream, but a PR's Job targets the Repo the PR is in"
        )
    );
    assert_eq!(tone, Tone::Plain);
    assert!(
        harness
            .view
            .footer
            .hints
            .iter()
            .any(|h| h.key == "u" && h.label == "upstream")
    );

    // The sheet's `u` can still choose the Upstream for this Job: refused,
    // with the existing explanation, and nothing done.
    press(&mut harness, Key::Char('u'));
    let refused = sheet(&harness).clone();
    assert_eq!(refused.status, LaunchStatus::Refused);
    assert_eq!(target(&refused).0, format!("upstream → {UPSTREAM}"));
    assert!(
        messages(&refused).contains(
            "PR #7 is in octo/app, not in upstream-org/app: den fetches a PR only from the Repo it is in"
        ),
        "{}",
        messages(&refused)
    );
    assert!(harness.workspace.calls().is_empty());

    // `u` again: back on the fork, and it launches there.
    press(&mut harness, Key::Char('u'));
    assert_eq!(sheet(&harness).status, LaunchStatus::Ready);
    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].target.repo, repo_id(FORK));
    assert!(store.stored().is_empty());
}

#[test]
fn issues_and_runs_follow_the_forks_upstream_choice_and_start_from_the_upstreams_default_branch() {
    // Decision b, the other side: Issues and Runs keep following the fork's
    // saved choice, and their new branches start from the Upstream's default
    // branch (Target.default_branch), fetched from the Upstream's remote.
    let store = Arc::new(FakeUpstreams::holding(targets(&[FORK])));
    let mut observation = forked();
    observation
        .runs
        .push(run(FORK, "ci.yml", 41).failed().by(ME).build());
    let mut harness = started_with(&store, observation);
    fork_clone(&harness);

    for reference in [ISSUE_9, "octo/app workflow .github/workflows/ci.yml"] {
        open_sheet(&mut harness, reference);
        let ready = sheet(&harness).clone();
        assert_eq!(
            ready.status,
            LaunchStatus::Ready,
            "{reference}: {}",
            messages(&ready)
        );
        assert_eq!(
            target(&ready).0,
            format!("upstream → {UPSTREAM}"),
            "{reference}"
        );
        let base = ready.fields.iter().find(|f| f.label == "BASE").unwrap();
        assert_eq!(base.value, "default branch main @ 2222222", "{reference}");
        press(&mut harness, Key::Enter);
        let plans = launch_plans(&harness);
        let plan = plans.last().expect("a plan");
        assert_eq!(plan.target.repo, repo_id(UPSTREAM), "{reference}");
        assert_eq!(plan.target.upstream_of, Some(repo_id(FORK)), "{reference}");
        assert_eq!(plan.remote.name, "upstream", "{reference}");
        assert_eq!(plan.base.commit, UPSTREAM_MAIN, "{reference}");
        press(&mut harness, Key::Esc);
    }
}

#[test]
fn a_branch_sheet_starts_on_the_fork_and_an_upstream_target_for_it_is_refused() {
    // A Branch's commits live in the fork, like a PR's: an Upstream target
    // would fetch whatever the Upstream has under that name. So its sheet
    // starts on the fork, and `u` to the Upstream is refused with the reason.
    let store = Arc::new(FakeUpstreams::holding(targets(&[FORK])));
    // Branches compared against another branch than their target's wait for a
    // refresh (#13), so this one is compared against the Upstream's default
    // branch, as a survey with the fork set to Upstream does.
    let observation = ObservationBuilder::new()
        .owner("octo")
        .fork(FORK, Some(UPSTREAM))
        .branch(
            branch(FORK, "my-fix")
                .compared_against(UPSTREAM, "main")
                .author(ME),
        )
        .build();
    let mut harness = started_with(&store, observation);
    fork_clone(&harness);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "my-fix", HEAD_7);
    // The Upstream has a branch of the same name: den must never use it.
    harness
        .workspace
        .set_remote_head(CLONE, "upstream", "my-fix", UPSTREAM_MAIN);

    open_sheet(&mut harness, "octo/app:my-fix");
    let fork_sheet = sheet(&harness).clone();
    assert_eq!(
        fork_sheet.status,
        LaunchStatus::Ready,
        "{}",
        messages(&fork_sheet)
    );
    let (value, note, _) = target(&fork_sheet);
    assert_eq!(value, FORK);
    assert!(
        note.unwrap()
            .ends_with("a Branch's Job targets the Repo the Branch is in")
    );
    press(&mut harness, Key::Char('u'));
    let refused = sheet(&harness).clone();
    assert_eq!(refused.status, LaunchStatus::Refused);
    assert!(
        messages(&refused).contains(
            "the Branch my-fix is in octo/app, not in upstream-org/app: den fetches a Branch only from the Repo it is in"
        ),
        "{}",
        messages(&refused)
    );
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn the_sheet_override_to_upstream_shows_on_the_sheet_and_leaves_the_fork_standalone() {
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, forked());
    fork_clone(&harness);
    open_sheet(&mut harness, PR_7);
    assert_eq!(sheet(&harness).status, LaunchStatus::Ready);

    press(&mut harness, Key::Char('u'));
    let (value, note, tone) = target(sheet(&harness));
    assert_eq!(value, format!("upstream → {UPSTREAM}"));
    assert_eq!(tone, Tone::Upstream);
    assert!(note.unwrap().contains("for this Job only"));
    assert!(stores(&harness.effects).is_empty());
    assert_eq!(row(&harness.view, &repo_scope(FORK)).upstream, None);
}

#[test]
fn u_on_a_repo_that_is_not_a_fork_changes_nothing() {
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, forked());
    for scope in [
        repo_scope("octo/lib"),
        Scope::Owner("octo".to_owned()),
        Scope::AllOwners,
    ] {
        sidebar_cursor_on(&mut harness, &scope);
        let before = harness.view.clone();
        let effects = harness.key(Key::Char('u'));
        assert!(effects.is_empty(), "{scope:?}: {effects:?}");
        assert_eq!(harness.view, before, "{scope:?}");
    }
    assert!(store.stored().is_empty());

    // On a non-fork's launch sheet, `u` changes nothing but says why.
    let plain = ObservationBuilder::new()
        .owner("octo")
        .repo("octo/lib")
        .pull_request(pr("octo/lib", 3).author(ME).changes_requested())
        .build();
    let mut harness = started_with(&store, plain);
    harness.workspace.add_clone(
        "/home/me/dev/lib",
        &[("origin", "https://github.com/octo/lib")],
    );
    harness.workspace.set_remote_head(
        "/home/me/dev/lib",
        "origin",
        "feature-3",
        &format!("{:040x}", 3),
    );
    press(&mut harness, Key::Char('c'));
    let before = sheet(&harness).clone();
    assert_eq!(before.status, LaunchStatus::Ready, "{}", messages(&before));
    let effects = harness.key(Key::Char('u'));
    assert!(effects.is_empty());
    let after = sheet(&harness);
    assert_eq!(after.fields, before.fields);
    assert!(
        messages(after).contains("u does nothing here: octo/lib is not a fork"),
        "{}",
        messages(after)
    );
}

#[test]
fn a_cross_repo_pr_head_is_fetched_by_its_pr_ref_from_the_target_repos_remote() {
    let store = Arc::new(FakeUpstreams::default());
    let observation = ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(
            pr("octo/app", 7)
                .author(ME)
                .changes_requested()
                .head_from("someone/app"),
        )
        .build();
    let mut harness = started_with(&store, observation);
    // The clone also has a remote for the head's Repo: den must not use it.
    harness.workspace.add_clone(
        CLONE,
        &[
            ("origin", "https://github.com/octo/app"),
            ("someone", "https://github.com/someone/app"),
        ],
    );
    harness
        .workspace
        .set_remote_head(CLONE, "someone", "feature-7", HEAD_7);
    harness
        .workspace
        .set_remote_pull_head(CLONE, "origin", 7, HEAD_7);

    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    let base = ready.fields.iter().find(|f| f.label == "BASE").unwrap();
    assert_eq!(base.value, "PR ref refs/pull/7/head @ 0000000");
    press(&mut harness, Key::Enter);

    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].remote.name, "origin");
    assert_eq!(plans[0].base.reference, BaseRef::PullHead { number: 7 });
    assert_eq!(plans[0].branch.start, BranchStart::NewUntracked);
    // PR refs use their own namespace so a remote branch named pr/7 is preserved.
    let calls = harness.workspace.calls();
    assert_eq!(
        calls[0],
        WorkspaceCall::Fetch {
            clone: CLONE.into(),
            remote: "origin".to_owned(),
            refspec: "+refs/pull/7/head:refs/den/pull/origin/7".to_owned(),
        }
    );
    assert_eq!(
        calls[1],
        WorkspaceCall::AddWorktree {
            clone: CLONE.into(),
            add: WorktreeAdd {
                path: "/home/me/dev/app/.claude/worktrees/feature-7".into(),
                branch: "feature-7".to_owned(),
                start: WorktreeStart::NewBranch {
                    start: "refs/den/pull/origin/7".to_owned()
                },
            },
        }
    );
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
}

#[test]
fn a_cross_repo_pr_head_with_no_main_clone_is_cloned_then_fetched_by_its_pr_ref() {
    // #12's clone step with #13's PR ref: verified against the URL before
    // anything is cloned, then fetched from the new clone's `origin`.
    let store = Arc::new(FakeUpstreams::default());
    let observation = ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(
            pr("octo/app", 7)
                .author(ME)
                .changes_requested()
                .head_from("someone/app"),
        )
        .build();
    let mut harness = started_with(&store, observation);
    harness
        .workspace
        .add_existing("/home/me/dev/other/notes.txt");
    harness
        .workspace
        .host_pull_head("https://github.com/octo/app", 7, HEAD_7);

    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    let base = ready.fields.iter().find(|f| f.label == "BASE").unwrap();
    assert_eq!(base.value, "PR ref refs/pull/7/head @ 0000000");
    press(&mut harness, Key::Enter);

    // PR refs use their own namespace so a remote branch named pr/7 is preserved.
    let calls = harness.workspace.calls();
    assert_eq!(
        calls[..2],
        [
            WorkspaceCall::Clone {
                remote: "origin".to_owned(),
                url: "https://github.com/octo/app".to_owned(),
                into: CLONE.into(),
            },
            WorkspaceCall::Fetch {
                clone: CLONE.into(),
                remote: "origin".to_owned(),
                refspec: "+refs/pull/7/head:refs/den/pull/origin/7".to_owned(),
            },
        ]
    );
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
}

/// octo/app (not a fork) with PR #7, mine, whose head branch `head` lives
/// in someone's fork.
fn cross_repo_pr(head: &str) -> Observation {
    ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(
            pr("octo/app", 7)
                .author(ME)
                .changes_requested()
                .head_from("someone/app")
                .head_ref(head),
        )
        .build()
}

#[test]
fn a_cross_repo_head_named_like_the_default_branch_is_checked_out_as_pr_n_head() {
    // Decision d: someone's fork opened PR #7 from their `main`; the Main
    // Clone has its own `main`, so the Job's branch is `pr-7-main`.
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, cross_repo_pr("main"));
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "https://github.com/octo/app")]);
    harness
        .workspace
        .set_local_branch(CLONE, "main", UPSTREAM_MAIN);
    harness
        .workspace
        .set_remote_pull_head(CLONE, "origin", 7, HEAD_7);

    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    let branch = ready.fields.iter().find(|f| f.label == "BRANCH").unwrap();
    assert_eq!(branch.value, "pr-7-main");
    assert!(
        branch
            .note
            .as_deref()
            .unwrap()
            .ends_with("main is taken in the Main Clone, so den names it pr-7-main"),
        "{:?}",
        branch.note
    );
    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].branch.name, "pr-7-main");
    assert_eq!(plans[0].branch.renamed_from.as_deref(), Some("main"));
    assert_eq!(plans[0].branch.start, BranchStart::NewUntracked);
    let add = harness
        .workspace
        .calls()
        .into_iter()
        .find_map(|c| match c {
            WorkspaceCall::AddWorktree { add, .. } => Some(add),
            _ => None,
        })
        .expect("a worktree added");
    assert_eq!(add.branch, "pr-7-main");
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
    // `main` itself was never touched.
    assert_eq!(harness.jobs.records()[0].branch, "pr-7-main");
}

#[test]
fn a_cross_repo_head_whose_name_is_already_a_branch_here_is_slugged_into_pr_n_head() {
    // Decision d: any name already present in the Main Clone clashes too; the
    // head branch is slugged (`Fix/Login Page` → `fix-login-page`).
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, cross_repo_pr("Fix/Login_Page"));
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "https://github.com/octo/app")]);
    harness
        .workspace
        .set_local_branch(CLONE, "Fix/Login_Page", UPSTREAM_MAIN);
    harness
        .workspace
        .set_remote_pull_head(CLONE, "origin", 7, HEAD_7);

    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans[0].branch.name, "pr-7-fix-login-page");
    assert_eq!(
        plans[0].branch.renamed_from.as_deref(),
        Some("Fix/Login_Page")
    );
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
}

#[test]
fn a_cross_repo_head_that_clashes_with_nothing_keeps_its_name() {
    // The boundary: `feature-7` is neither the default branch nor present.
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, cross_repo_pr("feature-7"));
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "https://github.com/octo/app")]);
    harness
        .workspace
        .set_local_branch(CLONE, "main", UPSTREAM_MAIN);
    harness
        .workspace
        .set_remote_pull_head(CLONE, "origin", 7, HEAD_7);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans[0].branch.name, "feature-7");
    assert_eq!(plans[0].branch.renamed_from, None);
}

#[test]
fn a_cross_repo_head_named_like_the_default_branch_is_renamed_in_a_clone_den_makes_too() {
    // No Main Clone: the clone den makes checks out the default branch, so
    // `main` clashes there too before anything is inspected locally.
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, cross_repo_pr("main"));
    harness
        .workspace
        .add_existing("/home/me/dev/other/notes.txt");
    harness
        .workspace
        .host_pull_head("https://github.com/octo/app", 7, HEAD_7);
    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans[0].branch.name, "pr-7-main");
    assert!(plans[0].clones());
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
}

#[test]
fn a_cross_repo_pr_ref_missing_from_the_target_remote_refuses_at_launch_doing_nothing() {
    let store = Arc::new(FakeUpstreams::default());
    let observation = ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(
            pr("octo/app", 7)
                .author(ME)
                .changes_requested()
                .head_from("someone/app"),
        )
        .build();
    let mut harness = started_with(&store, observation);
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "https://github.com/octo/app")]);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Enter);
    let refused = sheet(&harness);
    assert_eq!(refused.status, LaunchStatus::Refused);
    assert!(
        messages(refused).contains("origin has no refs/pull/7/head"),
        "{}",
        messages(refused)
    );
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn the_upstream_target_carries_the_upstream_label_as_well_as_the_rust_tone() {
    let store = Arc::new(FakeUpstreams::holding(targets(&[FORK])));
    let mut harness = started_with(&store, forked());
    // The sheet: rust tone and the text label (an Issue's sheet follows the
    // fork's setting; a PR's starts on the fork, decision b).
    open_sheet(&mut harness, ISSUE_9);
    let (value, _, tone) = target(sheet(&harness));
    assert!(value.starts_with("upstream → "), "{value}");
    assert_eq!(tone, Tone::Upstream);
    press(&mut harness, Key::Esc);
    // The sidebar row and the header's Scope sentence say it in text too.
    assert_eq!(
        row(&harness.view, &repo_scope(FORK)).upstream.as_deref(),
        Some(UPSTREAM)
    );
    sidebar_cursor_on(&mut harness, &repo_scope(FORK));
    press(&mut harness, Key::Enter);
    assert_eq!(
        harness.view.header.scope,
        format!("Repo · {FORK} · fork of {UPSTREAM} · upstream → {UPSTREAM}")
    );
}

#[test]
fn switching_the_target_puts_the_forks_branches_on_hold_until_a_refresh_compares_them_again() {
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, forked());
    assert_eq!(step_of(&harness, "my-fix"), "Open PR");

    sidebar_cursor_on(&mut harness, &repo_scope(FORK));
    press(&mut harness, Key::Char('u'));
    // Counted against the fork's own `main`, which is no longer the comparison.
    assert_ne!(step_of(&harness, "my-fix"), "Open PR");
    assert_eq!(harness.survey_effects(), 1, "no survey by itself");

    // The refresh asks GitHub to compare against the Upstream, and the answer
    // (compared there) gives the Branch its Next Step back.
    let compared_upstream = ObservationBuilder::new()
        .owner("octo")
        .fork(FORK, Some(UPSTREAM))
        .repo("octo/lib")
        .branch(
            branch(FORK, "my-fix")
                .compared_against(UPSTREAM, "main")
                .author(ME),
        )
        .build();
    harness.answer(Ok(compared_upstream));
    press(&mut harness, Key::Esc);
    press(&mut harness, Key::Char('r'));
    assert_eq!(harness.github.requests()[1].upstreams, targets(&[FORK]));
    assert_eq!(step_of(&harness, "my-fix"), "Open PR");
}

#[test]
fn a_fork_whose_upstream_den_cannot_see_is_not_switched() {
    let store = Arc::new(FakeUpstreams::default());
    let observation = ObservationBuilder::new()
        .owner("octo")
        .fork(FORK, None)
        .build();
    let mut harness = started_with(&store, observation);
    sidebar_cursor_on(&mut harness, &repo_scope(FORK));
    let effects = harness.key(Key::Char('u'));
    assert!(stores(&effects).is_empty());
    assert_eq!(row(&harness.view, &repo_scope(FORK)).upstream, None);
    assert!(
        harness
            .view
            .header
            .status
            .contains("Upstream is not visible"),
        "{}",
        harness.view.header.status
    );
}

#[test]
fn unreadable_upstream_choices_count_as_none_and_are_never_overwritten() {
    let error = UpstreamStoreError::Read {
        path: "/state/upstreams.json".into(),
        detail: "bad JSON".to_owned(),
    };
    let store = Arc::new(FakeUpstreams::failing_load(error));
    let mut harness = started_with(&store, forked());
    assert!(
        harness
            .view
            .header
            .status
            .contains("Upstream choices unreadable"),
        "{}",
        harness.view.header.status
    );
    assert_eq!(row(&harness.view, &repo_scope(FORK)).upstream, None);
    sidebar_cursor_on(&mut harness, &repo_scope(FORK));
    let effects = harness.key(Key::Char('u'));
    assert!(stores(&effects).is_empty(), "{effects:?}");
    assert_eq!(row(&harness.view, &repo_scope(FORK)).upstream, None);
}

#[test]
fn a_choice_that_could_not_be_saved_says_so() {
    let store = Arc::new(FakeUpstreams::default());
    store.fail_store(UpstreamStoreError::Write {
        path: "/state/upstreams.json".into(),
        detail: "disk full".to_owned(),
    });
    let mut harness = started_with(&store, forked());
    sidebar_cursor_on(&mut harness, &repo_scope(FORK));
    press(&mut harness, Key::Char('u'));
    assert!(
        harness
            .view
            .header
            .status
            .contains("Upstream choice not saved"),
        "{}",
        harness.view.header.status
    );
    // It still applies for this session.
    assert_eq!(
        row(&harness.view, &repo_scope(FORK)).upstream.as_deref(),
        Some(UPSTREAM)
    );
}

fn prompt_text(harness: &Harness) -> String {
    sheet(harness)
        .prompt
        .as_ref()
        .unwrap_or_else(|| panic!("no prompt: {:#?}", sheet(harness)))
        .text
        .clone()
}

#[test]
fn an_edited_prompt_survives_a_target_round_trip_and_an_undone_edit_stays_undone() {
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, forked());
    fork_clone(&harness);
    press(&mut harness, Key::Char('c'));
    let default_prompt = prompt_text(&harness);

    // Edit: append `!`.
    for key in [Key::Char('e'), Key::End, Key::Char('!'), Key::Esc] {
        press(&mut harness, key);
    }
    let edited_prompt = format!("{default_prompt}!");
    assert_eq!(prompt_text(&harness), edited_prompt);

    // To the Upstream and back: the edit is still there.
    press(&mut harness, Key::Char('u'));
    press(&mut harness, Key::Char('u'));
    assert_eq!(sheet(&harness).status, LaunchStatus::Ready);
    assert_eq!(prompt_text(&harness), edited_prompt);

    // Undo the edit by hand, then round-trip again: the default stays, and
    // the discarded `!` does not come back.
    for key in [Key::Char('e'), Key::End, Key::Backspace, Key::Esc] {
        press(&mut harness, key);
    }
    assert_eq!(prompt_text(&harness), default_prompt);
    press(&mut harness, Key::Char('u'));
    press(&mut harness, Key::Char('u'));
    assert_eq!(sheet(&harness).status, LaunchStatus::Ready);
    assert_eq!(prompt_text(&harness), default_prompt);
}

#[test]
fn an_edited_prompt_survives_a_moved_head_and_then_a_target_round_trip() {
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, forked());
    fork_clone(&harness);
    press(&mut harness, Key::Char('c'));
    for key in [Key::Char('e'), Key::End, Key::Char('!'), Key::Esc] {
        press(&mut harness, key);
    }
    let edited_prompt = prompt_text(&harness);
    assert!(edited_prompt.ends_with('!'));

    // The head moves before the launch: the sheet asks again, and the plan it
    // holds now carries the edited prompt.
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-7", &"1".repeat(40));
    press(&mut harness, Key::Enter);
    assert_eq!(sheet(&harness).status, LaunchStatus::HeadMoved);
    assert_eq!(prompt_text(&harness), edited_prompt);

    // To the Upstream and back: the edit is still the user's, not dropped as
    // if it were the default.
    press(&mut harness, Key::Char('u'));
    press(&mut harness, Key::Char('u'));
    assert_eq!(prompt_text(&harness), edited_prompt);
}

#[test]
fn starting_fresh_beside_a_live_job_keeps_the_sheets_upstream_target() {
    // The fork targets its Upstream; an Issue in the fork starts from the
    // Upstream's default branch.
    let store = Arc::new(FakeUpstreams::holding(targets(&[FORK])));
    let observation = ObservationBuilder::new()
        .owner("octo")
        .fork(FORK, Some(UPSTREAM))
        .issue(issue(FORK, 9).assignee(ME).label("feature"))
        .build();
    let mut harness = started_with(&store, observation);
    fork_clone(&harness);
    harness
        .workspace
        .set_remote_head(CLONE, "upstream", "main", UPSTREAM_MAIN);

    open_sheet(&mut harness, "octo/app#9");
    let first = sheet(&harness).clone();
    assert_eq!(first.status, LaunchStatus::Ready, "{}", messages(&first));
    assert_eq!(target(&first).0, format!("upstream → {UPSTREAM}"));
    press(&mut harness, Key::Enter);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
    press(&mut harness, Key::Esc);

    // Again: a Live Job. `f` starts fresh for the sheet's target, the Upstream,
    // not for the default target (the fork itself).
    open_sheet(&mut harness, "octo/app#9");
    assert_eq!(sheet(&harness).status, LaunchStatus::LiveJob);
    press(&mut harness, Key::Char('f'));
    let fresh = sheet(&harness).clone();
    assert_eq!(fresh.status, LaunchStatus::Ready, "{}", messages(&fresh));
    assert_eq!(target(&fresh).0, format!("upstream → {UPSTREAM}"));
    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[1].target.repo, repo_id(UPSTREAM));
    assert_eq!(plans[1].remote.name, "upstream");
    assert_eq!(plans[1].base.commit, UPSTREAM_MAIN);
    assert_ne!(plans[1].worktree, plans[0].worktree);
    assert_ne!(plans[1].branch.name, plans[0].branch.name);
}

#[test]
fn an_upstream_job_remains_reopenable_when_only_the_forks_owner_is_watched() {
    use den::domain::facts::{LinkKind, LinkedPrState};
    let observed = |linked| {
        let issue = issue(FORK, 9).assignee(ME).label("feature");
        let issue = if linked {
            issue.linked(
                LinkKind::ClosingReference,
                UPSTREAM,
                12,
                LinkedPrState::Open,
            )
        } else {
            issue
        };
        ObservationBuilder::new()
            .owner("octo")
            .fork(FORK, Some(UPSTREAM))
            .issue(issue)
            .build()
    };
    let store = Arc::new(FakeUpstreams::holding(targets(&[FORK])));
    let mut harness = started_with(&store, observed(false));
    fork_clone(&harness);
    open_sheet(&mut harness, ISSUE_9);
    press(&mut harness, Key::Enter);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
    let job = harness.jobs.records()[0].clone();
    assert_eq!(job.target, repo_id(UPSTREAM));
    press(&mut harness, Key::Esc);
    let updated = observed(true);
    assert!(updated.repo(&repo_id(UPSTREAM)).is_none());
    harness.answer(Ok(updated));
    press(&mut harness, Key::Char('r'));
    open_sheet(&mut harness, ISSUE_9);
    assert_eq!(sheet(&harness).next_step, "none");
    assert_eq!(
        sheet(&harness).status,
        LaunchStatus::LiveJob,
        "{}",
        messages(sheet(&harness))
    );
    press(&mut harness, Key::Char('o'));
    press(&mut harness, Key::Enter);
    assert_eq!(sheet(&harness).status, LaunchStatus::Reopened);
    assert_eq!(harness.launcher.windows()[1].session, job.session);
    assert_eq!(harness.jobs.records(), [job]);
}

#[test]
fn rejected_upstream_saves_are_visible_and_rejected_surveys_can_be_retried() {
    let store = Arc::new(FakeUpstreams::default());
    let mut harness = started_with(&store, forked());
    sidebar_cursor_on(&mut harness, &repo_scope(FORK));
    let effects = harness.send(Event::Key(Key::Char('u')));
    let save = effects
        .into_iter()
        .find(|e| matches!(e, Effect::StoreUpstreams { .. }))
        .unwrap();
    harness.send(Event::EffectRejected {
        effect: Box::new(save),
        reason: "worker busy".into(),
    });
    assert!(
        harness
            .view
            .header
            .status
            .contains("Upstream choice not saved")
    );
    assert!(harness.view.header.status.contains("worker busy"));
    assert!(store.stored().is_empty());
    let effects = harness.send(Event::Key(Key::Char('r')));
    let survey = effects
        .into_iter()
        .find(|e| matches!(e, Effect::Survey { .. }))
        .unwrap();
    harness.send(Event::EffectRejected {
        effect: Box::new(survey),
        reason: "worker busy".into(),
    });
    let retry = harness.send(Event::Key(Key::Char('r')));
    assert!(retry.iter().any(|e| matches!(e, Effect::Survey { .. })));
}

#[test]
fn differently_cased_head_repository_still_tracks_the_same_repository_branch() {
    let store = Arc::new(FakeUpstreams::default());
    let observed = ObservationBuilder::new()
        .owner("octo")
        .repo(FORK)
        .pull_request(
            pr(FORK, 7)
                .author(ME)
                .changes_requested()
                .head_from("OCTO/App"),
        )
        .build();
    let mut harness = started_with(&store, observed);
    fork_clone(&harness);
    open_sheet(&mut harness, PR_7);
    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(
        plans[0].base.reference,
        BaseRef::HeadBranch("feature-7".into())
    );
    assert_eq!(
        plans[0].branch.start,
        BranchStart::New {
            tracking: "origin/feature-7".into()
        }
    );
}
