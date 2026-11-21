//! Seam 2: Live Jobs (#14). den recognises the Jobs it launched: each recorded
//! Job's worktree is present, missing or unknown; the list and detail mark it;
//! `c` on a Live Job asks "open existing or start fresh?"; an unknown state is
//! explained and never relaunched silently; nothing is ever deleted.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Effect, Event, Key,
        view_model::{
            JobActivity, JobActivityLine, JobMarker, LaunchSheet, LaunchStatus, Overlay, ViewModel,
        },
    },
    config::Config,
    domain::{NextStep, RepoId, WorkItemId},
    handoff::{BranchStart, LaunchPlan, SessionStart, WindowCommand},
    jobs::{JobRecord, JobSubject},
    palette::OXIDE_TRUECOLOR,
    ui::{Theme, frame},
};
use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};
use std::{collections::BTreeMap, path::PathBuf};
use support::{
    build::{ME, ObservationBuilder, now, pr},
    fakes::WorkspaceCall,
    ground_rules::{coding_rules, quoted},
    harness::Harness,
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/app";
const WORKTREES: &str = "/home/me/dev/app/.claude/worktrees";
/// `pr()` gives PR n the head branch `feature-<n>` at the commit n in hex.
const HEAD_7: &str = "0000000000000000000000000000000000000007";
const SESSION_7: &str = "app-pr7-address-review";

fn worktree(n: u64) -> String {
    format!("{WORKTREES}/feature-{n}")
}

fn item(n: u64) -> WorkItemId {
    WorkItemId::PullRequest {
        repo: RepoId::new("octo", "app"),
        number: n,
    }
}

fn job(n: u64) -> JobRecord {
    JobRecord {
        subject: JobSubject::GitHub(item(n)),
        next_step: NextStep::AddressReview,
        target: RepoId::new("octo", "app"),
        main_clone: CLONE.into(),
        worktree: worktree(n).into(),
        branch: format!("feature-{n}"),
        session: format!("app-pr{n}-address-review"),
        launched_at: now(),
        permission_mode: None,
    }
}

/// octo/app with PRs 7, 8, 9 and 10, each Waiting On me to Address review.
fn observation() -> ObservationBuilder {
    let mut builder = ObservationBuilder::new().owner("octo").repo("octo/app");
    for n in 7..=10 {
        builder = builder.pull_request(pr("octo/app", n).author(ME).changes_requested());
    }
    builder
}

/// A harness with a Main Clone of octo/app (remote `upstream`); `arrange`
/// sets up Jobs and worktrees before den starts.
fn harness_with(arrange: impl FnOnce(&Harness)) -> Harness {
    harness_observing(observation(), arrange)
}

/// [`harness_with`], surveying `observed` instead.
fn harness_observing(observed: ObservationBuilder, arrange: impl FnOnce(&Harness)) -> Harness {
    let config = Config {
        owners: vec!["octo".to_owned()],
        main_clone_root: Some(PathBuf::from(ROOT)),
        main_clones: BTreeMap::new(),
        linear: Vec::new(),
        skips: den::domain::Skips::default(),
        screenshot_repos: Vec::new(),
    };
    let mut harness = Harness::new(Ok(config));
    harness
        .workspace
        .add_clone(CLONE, &[("upstream", "git@github.com:octo/app.git")]);
    for n in 7..=10 {
        harness.workspace.set_remote_head(
            CLONE,
            "upstream",
            &format!("feature-{n}"),
            &format!("{n:040x}"),
        );
    }
    arrange(&harness);
    harness.answer(Ok(observed.build()));
    harness.start();
    harness
}

/// A Live Job for PR `n`: recorded, its folder there, registered with git on
/// its branch (which the Job made and committed to).
fn live_job(harness: &Harness, n: u64) {
    harness.jobs.holding(job(n));
    worktree_in_use(harness, n);
}

/// PR `n`'s Job worktree: its folder there, registered with git on its branch
/// (which the Job made and committed to).
fn worktree_in_use(harness: &Harness, n: u64) {
    harness.workspace.add_existing(&worktree(n));
    harness
        .workspace
        .register_worktree(CLONE, &worktree(n), Some(&format!("feature-{n}")));
    harness.workspace.set_local_branch(
        CLONE,
        &format!("feature-{n}"),
        "2222222222222222222222222222222222222222",
    );
}

fn select(harness: &mut Harness, n: u64) {
    let target = item(n);
    let index = harness
        .view
        .list
        .rows
        .iter()
        .position(|r| r.id == target)
        .unwrap_or_else(|| panic!("PR {n} is listed"));
    let current = harness.view.list.selected.unwrap_or(0);
    let key = if index >= current { Key::Down } else { Key::Up };
    for _ in 0..index.abs_diff(current) {
        harness.key(key);
    }
    assert_eq!(harness.view.list.selected, Some(index));
}

fn press(harness: &mut Harness, key: Key) {
    harness.settle(Event::Key(key));
}

fn sheet(harness: &Harness) -> &LaunchSheet {
    match &harness.view.overlay {
        Some(Overlay::LaunchSheet(sheet)) => sheet,
        other => panic!("no launch sheet: {other:?}"),
    }
}

fn messages(sheet: &LaunchSheet) -> String {
    sheet
        .messages
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn field<'a>(sheet: &'a LaunchSheet, label: &str) -> (&'a str, Option<&'a str>) {
    let field = sheet
        .fields
        .iter()
        .find(|f| f.label == label)
        .unwrap_or_else(|| panic!("no {label} field: {:?}", sheet.fields));
    (field.value.as_str(), field.note.as_deref())
}

fn hints(view: &ViewModel) -> Vec<String> {
    view.footer
        .hints
        .iter()
        .map(|k| format!("{} {}", k.key, k.label))
        .collect()
}

fn marker(view: &ViewModel, n: u64) -> Option<JobMarker> {
    let target = item(n);
    view.list
        .rows
        .iter()
        .find(|r| r.id == target)
        .expect("listed")
        .job
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

fn launches_or_reopens(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Launch { .. } | Effect::Reopen { .. }))
        .count()
}

#[test]
fn present_missing_and_unknown_worktrees_each_show_in_the_list_and_the_detail() {
    let harness = &mut harness_with(|h| {
        live_job(h, 7);
        // PR 8's Job: its worktree is gone and git has nothing registered.
        h.jobs.holding(job(8));
        // PR 9's Job: git has its worktree locked.
        live_job(h, 9);
        h.workspace.flag_worktree(CLONE, &worktree(9), true, false);
        // PR 10 has no Job.
    });
    assert!(
        harness
            .effects
            .iter()
            .any(|e| matches!(e, Effect::CheckJobs)),
        "den checks the Jobs at start"
    );

    assert_eq!(marker(&harness.view, 7), Some(JobMarker::Live));
    assert_eq!(marker(&harness.view, 8), Some(JobMarker::Missing));
    assert_eq!(marker(&harness.view, 9), Some(JobMarker::Unknown));
    assert_eq!(marker(&harness.view, 10), None);

    let detail = |harness: &mut Harness, n: u64| {
        select(harness, n);
        harness
            .view
            .detail
            .content
            .as_ref()
            .expect("a selection")
            .job
            .clone()
    };
    let live = detail(harness, 7).expect("PR 7 has a Job");
    assert_eq!(live.marker, JobMarker::Live);
    assert!(live.summary.starts_with("Live Job"), "{}", live.summary);
    assert_eq!(
        live.lines,
        [
            worktree(7),
            format!("session {SESSION_7}, launched 2026-11-19 12:00 UTC"),
            "branch feature-7".to_owned(),
        ]
    );
    let gone = detail(harness, 8).expect("PR 8 has a Job");
    assert_eq!(gone.marker, JobMarker::Missing);
    assert!(gone.summary.contains("gone"), "{}", gone.summary);
    let unknown = detail(harness, 9).expect("PR 9 has a Job");
    assert_eq!(unknown.marker, JobMarker::Unknown);
    assert!(
        unknown.summary.contains("unknown") && unknown.summary.contains("locked"),
        "{}",
        unknown.summary
    );
    assert!(detail(harness, 10).is_none());
}

#[test]
fn each_way_den_cannot_inspect_a_worktree_is_unknown_never_missing() {
    let harness = harness_with(|h| {
        // Prunable: registered, but git says its folder is gone.
        h.jobs.holding(job(7));
        h.workspace
            .register_worktree(CLONE, &worktree(7), Some("feature-7"));
        h.workspace.flag_worktree(CLONE, &worktree(7), false, true);
        // The folder exists but git has no worktree registered there.
        h.jobs.holding(job(8));
        h.workspace.add_existing(&worktree(8));
        // The path can't be inspected.
        h.jobs.holding(job(9));
        h.workspace.make_unknown(&worktree(9), "permission denied");
        // A record pointing at the Main Clone itself, not a Job's worktree.
        h.jobs.holding(JobRecord {
            worktree: CLONE.into(),
            ..job(10)
        });
    });
    for n in 7..=10 {
        assert_eq!(marker(&harness.view, n), Some(JobMarker::Unknown), "PR {n}");
    }
}

#[test]
fn a_failed_worktree_listing_makes_an_existing_worktree_unknown() {
    let harness = harness_with(|h| {
        live_job(h, 7);
        h.workspace.fail("worktree-list", "git timed out after 30s");
    });
    assert_eq!(marker(&harness.view, 7), Some(JobMarker::Unknown));
}

#[test]
fn c_on_a_live_job_asks_open_existing_or_start_fresh_and_launches_nothing_by_itself() {
    let mut harness = harness_with(|h| live_job(h, 7));
    select(&mut harness, 7);

    press(&mut harness, Key::Char('c'));
    let asked = sheet(&harness).clone();
    assert_eq!(asked.status, LaunchStatus::LiveJob, "{}", messages(&asked));
    assert_eq!(
        asked.headline,
        "This Work Item has a Live Job: open existing or start fresh?"
    );
    assert_eq!(
        field(&asked, "JOB"),
        (
            worktree(7).as_str(),
            Some("present: it exists and git has it registered")
        )
    );
    assert_eq!(field(&asked, "SESSION").0, SESSION_7);
    assert_eq!(
        hints(&harness.view),
        ["o open existing", "f start fresh", "esc cancel"]
    );

    // Enter is not an answer: nothing launches.
    let effects = harness.key(Key::Enter);
    assert_eq!(launches_or_reopens(&effects), 0);
    assert_eq!(sheet(&harness).status, LaunchStatus::LiveJob);
    assert!(launch_plans(&harness).is_empty());
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn open_existing_resumes_the_recorded_session_in_the_existing_worktree_with_a_warning() {
    let mut harness = harness_with(|h| live_job(h, 7));
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));

    press(&mut harness, Key::Char('o'));
    let ready = sheet(&harness).clone();
    assert_eq!(
        ready.status,
        LaunchStatus::ReopenReady,
        "{}",
        messages(&ready)
    );
    assert_eq!(field(&ready, "WORKTREE").0, worktree(7));
    assert_eq!(field(&ready, "SESSION").0, SESSION_7);
    assert!(ready.prompt.is_none(), "a resumed session takes no prompt");
    assert_eq!(
        ready.command.as_deref(),
        Some(
            format!(
                "kitty --detach --hold --directory /home/me/dev/app/.claude/worktrees/feature-7 claude --append-system-prompt {} --resume app-pr7-address-review",
                quoted(&coding_rules("octo/app#7"))
            )
            .as_str()
        ),
        // Since #71 a resumed session gets its Ground Rules again, the same
        // as at launch (ADR-0008).
    );
    assert!(
        messages(&ready).contains(
            "If the Job's original window is still open, both windows will write to the same conversation."
        ),
        "{}",
        messages(&ready)
    );
    assert_eq!(hints(&harness.view), ["⏎ open", "esc cancel"]);

    press(&mut harness, Key::Enter);

    assert_eq!(
        harness.launcher.windows(),
        [WindowCommand {
            directory: worktree(7).into(),
            session: SESSION_7.to_owned(),
            prompt: String::new(),
            start: SessionStart::Resume,
            permission_mode: None,
            setting_sources: den::handoff::SettingSources::All,
            // No Linear Workflow configured: nothing to remove (#70).
            removed_env: Vec::new(),
            ground_rules: support::ground_rules::coding_rules("octo/app#7"),
        }]
    );
    // Nothing fetched, no worktree added, no new Job recorded, nothing deleted.
    assert!(harness.workspace.calls().is_empty());
    assert_eq!(harness.jobs.records(), [job(7)]);
    assert!(launch_plans(&harness).is_empty());
    let done = sheet(&harness);
    assert_eq!(done.status, LaunchStatus::Reopened, "{}", messages(done));
    assert!(
        messages(done).contains("opened a kitty window resuming session app-pr7-address-review"),
        "{}",
        messages(done)
    );
}

#[test]
fn reopening_rechecks_project_settings_when_a_survey_changes_the_viewer() {
    let mut harness = harness_with(|h| live_job(h, 7));
    select(&mut harness, 7);
    let survey = harness.key(Key::Char('r'));
    harness.answer(Ok(observation().viewer("alice").build()));
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Char('o'));
    assert_eq!(sheet(&harness).status, LaunchStatus::ReopenReady);
    assert!(
        !sheet(&harness)
            .command
            .as_ref()
            .unwrap()
            .contains("--setting-sources")
    );

    for effect in &survey {
        harness.perform(effect);
    }
    press(&mut harness, Key::Enter);

    // The old viewer trusted this PR; the new viewer must confirm the restricted command.
    assert!(harness.launcher.windows().is_empty());
    let ready = sheet(&harness);
    assert_eq!(ready.status, LaunchStatus::ReopenReady);
    assert!(messages(ready).contains("project settings changed"));
    assert!(field(ready, "PROJECT SETTINGS").0.starts_with("off"));
    assert!(
        ready
            .command
            .as_ref()
            .unwrap()
            // Since #71 the Ground Rules sit between them (ADR-0008).
            .contains("--setting-sources user --append-system-prompt ")
    );

    press(&mut harness, Key::Enter);
    assert_eq!(harness.launcher.windows().len(), 1);
    assert_eq!(
        harness.launcher.windows()[0].setting_sources,
        den::handoff::SettingSources::UserOnly
    );
    assert!(harness.workspace.calls().is_empty());
    assert_eq!(harness.jobs.records(), [job(7)]);
}

#[test]
fn open_existing_outside_kitty_copies_the_resume_command() {
    let mut harness = harness_with(|h| {
        live_job(h, 7);
        h.launcher.outside_kitty();
    });
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Char('o'));
    press(&mut harness, Key::Enter);
    assert_eq!(
        harness.launcher.clipboard(),
        // Since #71 the resume line carries the Ground Rules (ADR-0008).
        [format!(
            "cd /home/me/dev/app/.claude/worktrees/feature-7 && claude --append-system-prompt {} --resume app-pr7-address-review",
            quoted(&coding_rules("octo/app#7"))
        )]
    );
    assert!(harness.launcher.windows().is_empty());
}

#[test]
fn open_existing_refuses_when_the_worktree_vanished_after_the_sheet_asked() {
    let mut harness = harness_with(|h| live_job(h, 7));
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Char('o'));
    harness.workspace.remove_existing(&worktree(7));

    press(&mut harness, Key::Enter);
    let refused = sheet(&harness);
    assert_eq!(
        refused.status,
        LaunchStatus::Refused,
        "{}",
        messages(refused)
    );
    assert!(
        messages(refused).contains("is not a Live Job's any more"),
        "{}",
        messages(refused)
    );
    assert!(harness.launcher.windows().is_empty());
}

#[test]
fn open_existing_never_targets_the_main_clone_directory() {
    // A record (hand-edited, or from a bug) whose worktree is the Main Clone.
    let mut harness = harness_with(|h| {
        h.jobs.holding(JobRecord {
            worktree: CLONE.into(),
            ..job(7)
        });
    });
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));
    let sheet_now = sheet(&harness).clone();
    // Not a Live Job: there is no "open existing" to choose.
    assert_eq!(sheet_now.status, LaunchStatus::JobUnknown);
    assert!(
        field(&sheet_now, "JOB")
            .1
            .is_some_and(|n| n.contains("isn't inside /home/me/dev/app/.claude/worktrees")),
        "{:?}",
        sheet_now.fields
    );
    for key in [Key::Char('o'), Key::Enter] {
        let effects = harness.key(key);
        assert_eq!(launches_or_reopens(&effects), 0, "{key:?}");
    }
    assert!(harness.launcher.windows().is_empty());
    assert!(harness.launcher.clipboard().is_empty());
}

#[test]
fn start_fresh_plans_a_distinct_worktree_branch_and_session_and_leaves_the_old_one() {
    let mut harness = harness_with(|h| live_job(h, 7));
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));

    press(&mut harness, Key::Char('f'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    let fresh_worktree = format!("{WORKTREES}/feature-7-2");
    assert_eq!(
        field(&ready, "WORKTREE"),
        (fresh_worktree.as_str(), Some("free"))
    );
    assert_eq!(
        field(&ready, "BRANCH"),
        ("feature-7-2", Some("new, tracking upstream/feature-7"))
    );
    assert_eq!(field(&ready, "SESSION").0, "app-pr7-address-review-2");
    assert!(
        messages(&ready).contains(&format!(
            "Starting fresh: the Live Job's worktree {} is left as it is.",
            worktree(7)
        )),
        "{}",
        messages(&ready)
    );

    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1, "{plans:?}");
    let plan = &plans[0];
    assert_eq!(plan.worktree, PathBuf::from(&fresh_worktree));
    assert_eq!(plan.branch.name, "feature-7-2");
    assert_eq!(
        plan.branch.start,
        BranchStart::New {
            tracking: "upstream/feature-7".to_owned()
        }
    );
    assert_eq!(plan.base.commit, HEAD_7);
    // Only the new worktree was touched.
    let touched: Vec<PathBuf> = harness
        .workspace
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            WorkspaceCall::AddWorktree { add, .. } => Some(add.path),
            // A clone makes a folder too: it would count as touched.
            WorkspaceCall::Clone { into, .. } => Some(into),
            WorkspaceCall::Fetch { .. } => None,
        })
        .collect();
    assert_eq!(touched, [PathBuf::from(&fresh_worktree)]);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
    assert_eq!(harness.jobs.records().len(), 2);
    // Both Jobs are Live now; the list still marks the Work Item.
    assert_eq!(marker(&harness.view, 7), Some(JobMarker::Live));
}

#[test]
fn start_fresh_on_a_pr_ends_the_prompt_with_how_to_push_back_to_the_prs_branch() {
    // Decision c: the fresh Job's branch is `feature-7-2`, tracking
    // `upstream/feature-7`; a plain `git push` would refuse (push.default
    // simple), so the prompt's last line says how to push to the PR.
    let mut harness = harness_with(|h| live_job(h, 7));
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Char('f'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    let push_line = "Push with: git push upstream HEAD:feature-7";
    let shown = ready.prompt.as_ref().expect("a prompt").text.clone();
    assert!(shown.ends_with(push_line), "{shown}");

    press(&mut harness, Key::Enter);
    let plans = launch_plans(&harness);
    assert_eq!(plans.len(), 1, "{plans:?}");
    assert_eq!(
        plans[0].prompt,
        format!("/gh-resolve https://github.com/octo/app/pull/7 {push_line}")
    );
    assert!(plans[0].command.contains(push_line), "{}", plans[0].command);
}

#[test]
fn a_plain_launch_on_the_prs_own_branch_has_no_push_line() {
    // Without a fresh start the Job is on `feature-7` itself: `git push` works.
    let mut harness = harness_with(|_| {});
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness).clone();
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(&ready));
    let shown = ready.prompt.as_ref().expect("a prompt").text.clone();
    assert!(!shown.contains("Push with"), "{shown}");
}

#[test]
fn start_fresh_skips_suffixes_another_job_or_worktree_already_uses() {
    let mut harness = harness_with(|h| {
        live_job(h, 7);
        // An earlier fresh start, its worktree gone, still holds `-2` in the records.
        h.jobs.holding(JobRecord {
            worktree: format!("{WORKTREES}/feature-7-2").into(),
            branch: "feature-7-2".to_owned(),
            ..job(7)
        });
        // Something registered as `-3`.
        h.workspace
            .register_worktree(CLONE, &format!("{WORKTREES}/feature-7-3"), None);
    });
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Char('f'));
    assert_eq!(
        field(sheet(&harness), "WORKTREE").0,
        format!("{WORKTREES}/feature-7-4")
    );
}

#[test]
fn an_unknown_worktree_is_explained_offers_no_deletion_and_never_relaunches_silently() {
    let mut harness = harness_with(|h| {
        live_job(h, 7);
        h.workspace.flag_worktree(CLONE, &worktree(7), true, false);
    });
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));

    let explained = sheet(&harness).clone();
    assert_eq!(explained.status, LaunchStatus::JobUnknown);
    assert_eq!(
        explained.headline,
        "den can't tell whether this Work Item's Job still has its worktree"
    );
    assert_eq!(
        field(&explained, "JOB"),
        (
            worktree(7).as_str(),
            Some("unknown: git has the worktree locked")
        )
    );
    // The only choices: start fresh on purpose, or cancel. No deletion, no reopen.
    assert_eq!(hints(&harness.view), ["f start fresh", "esc cancel"]);
    assert!(
        messages(&explained).contains("won't treat it as gone, won't delete it"),
        "{}",
        messages(&explained)
    );
    for key in [Key::Enter, Key::Char('o'), Key::Char('d'), Key::Char('x')] {
        let effects = harness.key(key);
        assert_eq!(launches_or_reopens(&effects), 0, "{key:?}");
        assert_eq!(sheet(&harness).status, LaunchStatus::JobUnknown, "{key:?}");
    }
    assert!(harness.workspace.calls().is_empty());
    assert!(harness.launcher.windows().is_empty());

    // Asked explicitly: a fresh start beside it.
    press(&mut harness, Key::Char('f'));
    assert_eq!(sheet(&harness).status, LaunchStatus::Ready);
    assert_eq!(
        field(sheet(&harness), "WORKTREE").0,
        format!("{WORKTREES}/feature-7-2")
    );
}

/// octo/app with PR 7 waiting for its checks: no rule matches, so it has no
/// Next Step (spec, "Next Step rules").
fn waiting_for_checks() -> ObservationBuilder {
    ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(pr("octo/app", 7).author(ME).pending("ci"))
}

#[test]
fn a_live_job_stays_reopenable_while_its_pr_has_no_next_step() {
    let mut harness = harness_observing(waiting_for_checks(), |h| live_job(h, 7));
    select(&mut harness, 7);
    assert_eq!(marker(&harness.view, 7), Some(JobMarker::Live));

    press(&mut harness, Key::Char('c'));
    let asked = sheet(&harness).clone();
    assert_eq!(asked.next_step, "none");
    assert_eq!(asked.status, LaunchStatus::LiveJob, "{}", messages(&asked));
    assert_eq!(
        asked.headline,
        "This Work Item has a Live Job: open existing?"
    );
    assert_eq!(field(&asked, "SESSION").0, SESSION_7);
    // A fresh launch needs a Next Step, so `f` is not offered, and the sheet says why.
    assert_eq!(hints(&harness.view), ["o open existing", "esc cancel"]);
    assert!(
        messages(&asked)
            .contains("Starting fresh isn't possible: this Work Item has no Next Step to hand off"),
        "{}",
        messages(&asked)
    );

    // `f` anyway: refused with the reason, and the question stays open.
    let effects = harness.key(Key::Char('f'));
    assert_eq!(launches_or_reopens(&effects), 0);
    let still = sheet(&harness).clone();
    assert_eq!(still.status, LaunchStatus::LiveJob);
    assert!(
        messages(&still).contains("can't start fresh: this Work Item has no Next Step"),
        "{}",
        messages(&still)
    );

    press(&mut harness, Key::Char('o'));
    assert_eq!(sheet(&harness).status, LaunchStatus::ReopenReady);
    press(&mut harness, Key::Enter);
    assert_eq!(
        harness.launcher.windows(),
        [WindowCommand {
            directory: worktree(7).into(),
            session: SESSION_7.to_owned(),
            prompt: String::new(),
            start: SessionStart::Resume,
            permission_mode: None,
            setting_sources: den::handoff::SettingSources::All,
            // No Linear Workflow configured: nothing to remove (#70).
            removed_env: Vec::new(),
            ground_rules: support::ground_rules::coding_rules("octo/app#7"),
        }]
    );
    assert_eq!(sheet(&harness).status, LaunchStatus::Reopened);
    // Nothing fetched or added, no new Job recorded.
    assert!(harness.workspace.calls().is_empty());
    assert_eq!(harness.jobs.records(), [job(7)]);
}

/// Reopening a Live Job whose fresh launch is refused inspects the Work Item
/// asked about, as surveyed, not as the Job recorded it: the request belongs
/// to the sheet's item, whatever the Job's subject is (a Landing Group's Job
/// covers its members, #40).
#[test]
fn a_reopen_inspection_is_for_the_work_item_asked_about() {
    let observed = ObservationBuilder::new()
        .owner("OCTO")
        .repo("OCTO/App")
        .pull_request(pr("OCTO/App", 7).author(ME).pending("ci"));
    let mut harness = harness_observing(observed, |h| live_job(h, 7));
    let effects = harness.send(Event::Key(Key::Char('c')));
    let Some(Effect::InspectLaunch { request, .. }) = effects
        .into_iter()
        .find(|e| matches!(e, Effect::InspectLaunch { .. }))
    else {
        panic!("c on a Live Job inspects it");
    };
    assert_eq!(
        request.item,
        WorkItemId::PullRequest {
            repo: RepoId::new("OCTO", "App"),
            number: 7,
        }
    );
    assert_eq!(request.worktree_name, "feature-7");
}

#[test]
fn without_a_next_step_a_job_that_is_no_longer_live_is_refused_as_before() {
    let mut harness = harness_observing(waiting_for_checks(), |h| live_job(h, 7));
    select(&mut harness, 7);
    // The worktree goes after den last checked the Jobs: the sheet's own
    // inspection finds it missing, so only the fresh launch's refusal is left.
    harness.workspace.remove_existing(&worktree(7));

    press(&mut harness, Key::Char('c'));
    let refused = sheet(&harness);
    assert_eq!(
        refused.status,
        LaunchStatus::Refused,
        "{}",
        messages(refused)
    );
    assert!(
        messages(refused).contains("this Work Item has no Next Step to hand off"),
        "{}",
        messages(refused)
    );
    assert!(harness.launcher.windows().is_empty());
    assert!(harness.workspace.calls().is_empty());
}

#[test]
fn a_job_whose_worktree_is_gone_launches_as_usual() {
    let mut harness = harness_with(|h| h.jobs.holding(job(7)));
    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));
    let ready = sheet(&harness);
    assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(ready));
    assert_eq!(
        field(ready, "WORKTREE"),
        (worktree(7).as_str(), Some("free"))
    );
    assert_eq!(field(ready, "SESSION").0, SESSION_7);
}

#[test]
fn a_launch_marks_its_work_item_live_once_the_jobs_are_checked_again() {
    let mut harness = harness_with(|_| {});
    select(&mut harness, 7);
    assert_eq!(marker(&harness.view, 7), None);
    press(&mut harness, Key::Char('c'));
    press(&mut harness, Key::Enter);
    assert_eq!(sheet(&harness).status, LaunchStatus::Launched);
    assert_eq!(marker(&harness.view, 7), Some(JobMarker::Live));
}

#[test]
fn the_live_job_marker_is_a_teal_glyph_before_the_title_and_unknown_is_amber() {
    let mut harness = harness_with(|h| {
        live_job(h, 7);
        live_job(h, 9);
        h.workspace.flag_worktree(CLONE, &worktree(9), true, false);
    });
    let (cols, rows) = (160, 45);
    harness.send(Event::Resized { cols, rows });
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::TrueColor,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    // Only the list plate: the sidebar has glyphs of its own.
    let list = frame::regions(buffer.area, &harness.view.layout).list;
    let cell_of = |glyph: &str| {
        (list.top()..list.bottom())
            .flat_map(|y| (list.left()..list.right()).map(move |x| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == glyph)
            .unwrap_or_else(|| panic!("{glyph} is drawn:\n{}", frame::buffer_text(&buffer)))
    };
    assert_eq!(buffer[cell_of("●")].fg, OXIDE_TRUECOLOR.accent);
    assert_eq!(buffer[cell_of("◌")].fg, OXIDE_TRUECOLOR.warning);

    // ASCII and no colour still carry it, as a glyph.
    let mono = Theme::new(TerminalProfile::new(
        ColorProfile::NoColor,
        GlyphProfile::Ascii,
    ))
    .expect("theme");
    let text =
        frame::buffer_text(&frame::render(&mono, &harness.view, cols, rows).expect("renders"));
    let title_7 = &harness
        .view
        .list
        .rows
        .iter()
        .find(|r| r.id == item(7))
        .expect("listed")
        .title;
    assert!(text.contains(&format!("* {title_7}")), "{text}");
}

#[test]
fn fresh_push_instructions_preserve_shell_metacharacters_in_remote_and_branch() {
    for branch in [
        "fix/$USER",
        "fix/quote'",
        "fix/$(whoami)",
        "fix/semicolon;name",
        "fix/it's;$(printf-expanded)",
    ] {
        let remote = "up$stream";
        let observed = ObservationBuilder::new()
            .owner("octo")
            .repo("octo/app")
            .pull_request(
                pr("octo/app", 7)
                    .author(ME)
                    .changes_requested()
                    .head_ref(branch),
            );
        let mut harness = harness_observing(observed, |h| {
            h.workspace
                .add_clone(CLONE, &[(remote, "git@github.com:octo/app.git")]);
            h.workspace.set_remote_head(CLONE, remote, branch, HEAD_7);
            live_job(h, 7);
        });
        press(&mut harness, Key::Char('c'));
        press(&mut harness, Key::Char('f'));
        let ready = sheet(&harness);
        assert_eq!(ready.status, LaunchStatus::Ready, "{}", messages(ready));
        let prompt = &ready.prompt.as_ref().unwrap().text;
        let command = prompt.split_once("Push with: ").unwrap().1;
        // Parse the displayed instruction with a real shell, replacing git with an argv recorder.
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("git() {{ printf '%s\\n' \"$@\"; }}; {command}"))
            .env("USER", "unexpected-expansion")
            .output()
            .unwrap();
        assert!(output.status.success(), "{command}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("push\n{remote}\nHEAD:{branch}\n")
        );
    }
}

#[test]
fn repo_casing_changes_preserve_live_markers_and_reopening() {
    for no_next_step in [false, true] {
        let pr = pr("OCTO/App", 7).author(ME);
        let pr = if no_next_step {
            pr.pending("ci")
        } else {
            pr.changes_requested()
        };
        let observed = ObservationBuilder::new()
            .owner("OCTO")
            .repo("OCTO/App")
            .pull_request(pr);
        let mut harness = harness_observing(observed, |h| live_job(h, 7));
        assert_eq!(harness.view.list.rows[0].job, Some(JobMarker::Live));
        press(&mut harness, Key::Char('c'));
        assert_eq!(
            sheet(&harness).status,
            LaunchStatus::LiveJob,
            "{}",
            messages(sheet(&harness))
        );
        press(&mut harness, Key::Char('o'));
        press(&mut harness, Key::Enter);
        assert_eq!(sheet(&harness).status, LaunchStatus::Reopened);
        assert_eq!(harness.launcher.windows()[0].session, SESSION_7);
        assert!(harness.workspace.calls().is_empty());
    }
}

#[test]
fn rejected_inspections_launches_and_reopens_leave_the_pending_state() {
    for reopen in [false, true] {
        let mut harness = harness_with(|h| {
            if reopen {
                live_job(h, 7);
            }
        });
        select(&mut harness, 7);
        let effects = harness.send(Event::Key(Key::Char('c')));
        let inspect = effects
            .into_iter()
            .find(|e| matches!(e, Effect::InspectLaunch { .. }))
            .unwrap();
        assert_eq!(sheet(&harness).status, LaunchStatus::Inspecting);
        harness.send(Event::EffectRejected {
            effect: Box::new(inspect),
            reason: "worker busy".into(),
        });
        assert_eq!(sheet(&harness).status, LaunchStatus::Refused);
        assert!(messages(sheet(&harness)).contains("worker busy"));
        press(&mut harness, Key::Esc);
        press(&mut harness, Key::Char('c'));
        if reopen {
            press(&mut harness, Key::Char('o'));
        }
        let effects = harness.send(Event::Key(Key::Enter));
        let launch = effects
            .into_iter()
            .find(|e| matches!(e, Effect::Launch { .. } | Effect::Reopen { .. }))
            .unwrap();
        harness.send(Event::EffectRejected {
            effect: Box::new(launch),
            reason: "worker stopped".into(),
        });
        assert_eq!(sheet(&harness).status, LaunchStatus::Refused);
        assert!(messages(sheet(&harness)).contains("worker stopped"));
        assert!(harness.launcher.windows().is_empty());
        assert!(harness.workspace.calls().is_empty());
        press(&mut harness, Key::Esc);
        press(&mut harness, Key::Char('c'));
        assert_eq!(
            sheet(&harness).status,
            if reopen {
                LaunchStatus::LiveJob
            } else {
                LaunchStatus::Ready
            }
        );
    }
}

#[test]
fn repo_casing_does_not_create_a_collision_but_branch_casing_still_does() {
    use den::handoff::{JobCheck, WorktreeState, live, names};
    let mut recorded = job(7);
    recorded.subject = JobSubject::GitHub(WorkItemId::Branch {
        repo: RepoId::new("octo", "app"),
        name: "Feature".into(),
    });
    let checks = [JobCheck {
        job: recorded.clone(),
        state: WorktreeState::Present,
        activity: None,
    }];
    for (name, same) in [("Feature", true), ("feature", false)] {
        let current = WorkItemId::Branch {
            repo: RepoId::new("OCTO", "App"),
            name: name.into(),
        };
        assert_eq!(live::most_telling(&checks, &current).is_some(), same);
        let free = names::free_worktree_name(
            "feature-7",
            &PathBuf::from(CLONE),
            &current,
            false,
            &[recorded.clone()],
        )
        .unwrap();
        assert_eq!(free.name, if same { "feature-7" } else { "feature-7-2" });
    }
}

#[test]
fn casing_changes_keep_only_the_latest_record_for_each_job_worktree() {
    let harness = harness_with(|h| live_job(h, 7));
    let old = job(7);
    let mut latest = old.clone();
    latest.subject = JobSubject::GitHub(WorkItemId::PullRequest {
        repo: RepoId::new("OCTO", "App"),
        number: 7,
    });
    latest.session = "newer-session".into();
    let checked = den::handoff::live::check_jobs(
        &[old, latest.clone()],
        harness.workspace.as_ref(),
        std::time::Duration::from_secs(1),
    );
    assert_eq!(checked.checks.len(), 1);
    assert_eq!(checked.checks[0].job, latest);
}

/// The Job records milestone 1 wrote (`tests/fixtures/milestone-1/jobs.json`,
/// format 1), as today's store loads them.
fn milestone_1_records() -> Vec<JobRecord> {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(den::jobs::JOBS_FILE);
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/milestone-1/jobs.json"),
        &path,
    )
    .unwrap();
    den::ports::JobStore::load(&den::jobs::FileJobStore::new(path)).unwrap()
}

#[test]
fn a_job_milestone_1_recorded_is_still_recognised_as_a_live_job() {
    let records = milestone_1_records();
    // The fixture's PR Job was recorded with exactly `job(7)`'s values.
    assert_eq!(records[0], job(7));
    let mut harness = harness_with(|h| {
        for record in records {
            h.jobs.holding(record);
        }
        worktree_in_use(h, 7);
    });
    assert_eq!(marker(&harness.view, 7), Some(JobMarker::Live));
    assert_eq!(marker(&harness.view, 8), None);

    select(&mut harness, 7);
    press(&mut harness, Key::Char('c'));
    let asked = sheet(&harness);
    assert_eq!(asked.status, LaunchStatus::LiveJob, "{}", messages(asked));
}

// ---- #80: a Live Job's session running or idle (display only) ----

fn activity(view: &ViewModel, n: u64) -> Option<JobActivity> {
    let target = item(n);
    view.list
        .rows
        .iter()
        .find(|r| r.id == target)
        .expect("listed")
        .job_activity
}

/// The detail's line on PR `n`'s Job session, once it is selected.
fn activity_line(harness: &mut Harness, n: u64) -> Option<JobActivityLine> {
    select(harness, n);
    harness
        .view
        .detail
        .content
        .as_ref()
        .expect("a detail")
        .job
        .as_ref()
        .expect("a Job section")
        .activity
        .clone()
}

/// A `claude` process for `session`, as den starts one.
const CLAUDE: &str = "/opt/claude-code/bin/claude";

#[test]
fn live_jobs_show_running_or_idle_in_the_list_and_the_detail() {
    let mut harness = harness_with(|h| {
        live_job(h, 7);
        live_job(h, 8);
        // PR 9's Job: recorded, its worktree gone.
        h.jobs.holding(job(9));
        h.workspace.set_processes(&[
            &[
                CLAUDE,
                "-n",
                SESSION_7,
                "/gh-resolve https://github.com/octo/app/pull/7",
            ],
            // Claude Desktop is a `claude` too, without a session flag.
            &["/usr/lib/claude-desktop/claude", "--type=renderer"],
            // Another program naming PR 8's session is not its Claude Code.
            &["vim", "-n", "app-pr8-address-review"],
        ]);
    });

    assert_eq!(marker(&harness.view, 7), Some(JobMarker::Live));
    assert_eq!(activity(&harness.view, 7), Some(JobActivity::Running));
    assert_eq!(marker(&harness.view, 8), Some(JobMarker::Live));
    assert_eq!(activity(&harness.view, 8), Some(JobActivity::Idle));
    assert_eq!(marker(&harness.view, 9), Some(JobMarker::Missing));
    assert_eq!(activity(&harness.view, 9), None, "only a Live Job has one");
    assert_eq!(activity(&harness.view, 10), None, "no Job");

    assert_eq!(
        activity_line(&mut harness, 7),
        Some(JobActivityLine {
            activity: JobActivity::Running,
            text: format!("Session running: a local claude process has session {SESSION_7}"),
        })
    );
    assert_eq!(
        activity_line(&mut harness, 8),
        Some(JobActivityLine {
            activity: JobActivity::Idle,
            text: "Session idle: no local claude process has session app-pr8-address-review; it finished or stopped".to_owned(),
        })
    );
}

#[test]
fn a_reopened_session_resumed_by_name_is_running() {
    let harness = harness_with(|h| {
        live_job(h, 7);
        h.workspace.set_processes(&[&[
            "claude",
            "--permission-mode",
            "manual",
            "--resume",
            SESSION_7,
        ]]);
    });
    assert_eq!(activity(&harness.view, 7), Some(JobActivity::Running));
}

#[test]
fn an_unreadable_process_list_is_unknown_and_the_job_stays_live() {
    let mut harness = harness_with(|h| {
        live_job(h, 7);
        h.workspace
            .processes_unreadable("can't list /proc: permission denied");
    });
    assert_eq!(marker(&harness.view, 7), Some(JobMarker::Live));
    assert_eq!(activity(&harness.view, 7), Some(JobActivity::Unknown));
    assert_eq!(
        activity_line(&mut harness, 7),
        Some(JobActivityLine {
            activity: JobActivity::Unknown,
            text: "Running or idle: unknown, den couldn't read the local processes: can't list /proc: permission denied".to_owned(),
        })
    );
}

#[test]
fn running_or_idle_never_changes_whether_a_launch_asks_about_the_live_job() {
    for processes in [
        // Idle: the session ended or crashed, but the worktree is still there.
        vec![],
        vec![[CLAUDE, "-n", SESSION_7]],
    ] {
        let mut harness = harness_with(|h| {
            live_job(h, 7);
            let argvs: Vec<&[&str]> = processes.iter().map(|p| p.as_slice()).collect();
            h.workspace.set_processes(&argvs);
        });
        select(&mut harness, 7);
        press(&mut harness, Key::Char('c'));
        let asked = sheet(&harness);
        assert_eq!(asked.status, LaunchStatus::LiveJob, "{}", messages(asked));
        assert!(launch_plans(&harness).is_empty());
        assert!(harness.workspace.calls().is_empty());
    }
}

#[test]
fn running_and_idle_live_jobs_are_teal_glyphs_of_their_own_before_the_title() {
    let mut harness = harness_with(|h| {
        live_job(h, 7);
        live_job(h, 8);
        h.workspace.set_processes(&[&[CLAUDE, "-n", SESSION_7]]);
    });
    let (cols, rows) = (160, 45);
    harness.send(Event::Resized { cols, rows });
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::TrueColor,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    let list = frame::regions(buffer.area, &harness.view.layout).list;
    let cell_of = |glyph: &str| {
        (list.top()..list.bottom())
            .flat_map(|y| (list.left()..list.right()).map(move |x| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == glyph)
            .unwrap_or_else(|| panic!("{glyph} is drawn:\n{}", frame::buffer_text(&buffer)))
    };
    assert_eq!(buffer[cell_of("▸")].fg, OXIDE_TRUECOLOR.accent, "running");
    assert_eq!(buffer[cell_of("◐")].fg, OXIDE_TRUECOLOR.accent, "idle");

    let mono = Theme::new(TerminalProfile::new(
        ColorProfile::NoColor,
        GlyphProfile::Ascii,
    ))
    .expect("theme");
    let text =
        frame::buffer_text(&frame::render(&mono, &harness.view, cols, rows).expect("renders"));
    let title = |n: u64| {
        harness
            .view
            .list
            .rows
            .iter()
            .find(|r| r.id == item(n))
            .expect("listed")
            .title
            .clone()
    };
    assert!(text.contains(&format!("@ {}", title(7))), "{text}");
    assert!(text.contains(&format!(". {}", title(8))), "{text}");

    // The detail says it in words too.
    select(&mut harness, 7);
    let text =
        frame::buffer_text(&frame::render(&theme, &harness.view, cols, rows).expect("renders"));
    // The detail pane wraps the line; its start is on one row.
    assert!(text.contains("▸ Session running: a local claude"), "{text}");
}
