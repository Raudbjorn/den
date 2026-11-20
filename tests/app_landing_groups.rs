//! Seam 2, Landing Groups (#35): each Pull Request row names the Linear
//! Issue it belongs to ("◇ TEAM-1"), is possibly for ("possibly for
//! TEAM-3"), or is ambiguous between; the Linear Issue's detail lists every
//! member with Repo, number, state, own Next Step and Live Job marker; a
//! Repo Scope shows the Linear Issues with a member there; all of it reads
//! in monochrome.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        Event, Key,
        view_model::{DetailContent, JobMarker, RowLanding, ViewModel},
    },
    config::Config,
    domain::{
        Coverage, LinearPart, LinearSourceKind, NextStep, RepoId, Source, SourceKind, SourceState,
        WorkItemId, WorkItemKind,
        facts::{LinearIssue, ReviewVerdict},
    },
    jobs::{JobRecord, JobSubject},
    ordering::Scope,
    ports::LinearSurvey,
    ui::{Theme, frame},
};
use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};
use std::path::PathBuf;
use support::{
    build::{ME, ObservationBuilder, linear_issue, linear_viewer, now, pr},
    harness::{Harness, linear_workflow},
};

const ROOT: &str = "/home/me/dev";
const CLONE: &str = "/home/me/dev/app";
const WORKTREE_7: &str = "/home/me/dev/app/.claude/worktrees/feature-7";
const SIZE: (u16, u16) = (160, 45);

fn pr_id(repo: &str, number: u64) -> WorkItemId {
    let (owner, name) = repo.split_once('/').unwrap();
    WorkItemId::PullRequest {
        repo: RepoId::new(owner, name),
        number,
    }
}

/// octo (with a Linear Workflow on workspace `acme`) and its `app` and `api`
/// Repos:
///
/// - TEAM-1 has members app #7 (Address review, with a Live Job) and
///   api #8 (draft), and a link that isn't a PR;
/// - TEAM-2 shares app #7 with TEAM-1, so #7 is ambiguous;
/// - api #9's branch `team-3-cache` only hints at TEAM-3.
fn harness() -> Harness {
    harness_with(Vec::new())
}

/// [`harness`], with `extra` Linear Issues read beside TEAM-1 to TEAM-3.
fn harness_with(extra: Vec<LinearIssue>) -> Harness {
    let config = Config {
        owners: vec!["octo".to_owned()],
        main_clone_root: Some(PathBuf::from(ROOT)),
        main_clones: std::collections::BTreeMap::new(),
        linear: vec![linear_workflow("octo", "acme")],
    };
    let mut harness = Harness::new(Ok(config));
    let (cols, rows) = SIZE;
    harness.send(Event::Resized { cols, rows });
    harness
        .workspace
        .add_clone(CLONE, &[("upstream", "git@github.com:octo/app.git")]);
    harness.jobs.holding(JobRecord {
        subject: JobSubject::GitHub(pr_id("octo/app", 7)),
        next_step: NextStep::AddressReview,
        permission_mode: None,
        target: RepoId::new("octo", "app"),
        main_clone: CLONE.into(),
        worktree: WORKTREE_7.into(),
        branch: "feature-7".to_owned(),
        session: "app-pr7-address-review".to_owned(),
        launched_at: now(),
    });
    harness.workspace.add_existing(WORKTREE_7);
    harness
        .workspace
        .register_worktree(CLONE, WORKTREE_7, Some("feature-7"));
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("octo")
        .repo("octo/app")
        .repo("octo/api")
        // octo has a Linear Workflow, so #36's review rules apply: Address
        // review needs the change request at the current head.
        .pull_request(
            pr("octo/app", 7)
                .author(ME)
                .changes_requested()
                .review_at_head("reviewer", ReviewVerdict::ChangesRequested),
        )
        .pull_request(pr("octo/api", 8).author(ME).draft())
        .pull_request(pr("octo/api", 9).author(ME).head_ref("team-3-cache"))
        .build()));
    let mut coverage = Coverage::default();
    for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
        coverage.record(
            Source::Owner {
                owner: "octo".to_owned(),
                kind: SourceKind::Linear(kind),
            },
            SourceState::Complete,
        );
    }
    harness.linear.push_answer(LinearSurvey {
        part: LinearPart {
            viewers: vec![linear_viewer("octo", "acme")],
            issues: vec![
                linear_issue("octo", "id-1", "TEAM-1")
                    .attached("octo/app", 7)
                    .attached("octo/api", 8)
                    .link("https://www.figma.com/file/abc")
                    .build(),
                linear_issue("octo", "id-2", "TEAM-2")
                    .attached("octo/app", 7)
                    .build(),
                linear_issue("octo", "id-3", "TEAM-3").build(),
            ]
            .into_iter()
            .chain(extra)
            .collect(),
            ..LinearPart::default()
        },
        coverage,
    });
    harness.start();
    harness
}

fn row_landing(view: &ViewModel, id: &WorkItemId) -> RowLanding {
    view.list
        .rows
        .iter()
        .find(|r| &r.id == id)
        .unwrap_or_else(|| panic!("{id} is listed"))
        .landing
        .clone()
}

/// Selects the row whose reference is `reference` and returns its detail.
fn detail_of(harness: &mut Harness, reference: &str) -> DetailContent {
    let index = harness
        .view
        .list
        .rows
        .iter()
        .position(|r| r.reference == reference)
        .unwrap_or_else(|| panic!("{reference} is listed"));
    let current = harness.view.list.selected.unwrap_or(0);
    let key = if index >= current { Key::Down } else { Key::Up };
    for _ in 0..index.abs_diff(current) {
        harness.key(key);
    }
    harness.view.detail.content.clone().expect("selected")
}

/// Story 20: each PR row says which Linear Issue it belongs to; a branch
/// name only says "possibly for"; a PR in two groups says so.
#[test]
fn each_pr_row_names_its_landing_groups() {
    let harness = harness();
    let view = &harness.view;
    assert_eq!(
        row_landing(view, &pr_id("octo/app", 7)),
        RowLanding {
            groups: vec!["TEAM-1".to_owned(), "TEAM-2".to_owned()],
            ambiguous: true,
            possibly_for: Vec::new(),
        }
    );
    assert_eq!(
        row_landing(view, &pr_id("octo/api", 8)),
        RowLanding {
            groups: vec!["TEAM-1".to_owned()],
            ambiguous: false,
            possibly_for: Vec::new(),
        }
    );
    assert_eq!(
        row_landing(view, &pr_id("octo/api", 9)),
        RowLanding {
            groups: Vec::new(),
            ambiguous: false,
            possibly_for: vec!["TEAM-3".to_owned()],
        }
    );
}

/// Story 21: the Linear Issue's detail lists every member with its Repo,
/// number, state, own Next Step and Live Job marker; and the issue has no
/// Next Step of its own while they carry the work.
#[test]
fn a_linear_issue_detail_lists_every_member() {
    let mut harness = harness();
    let detail = detail_of(&mut harness, "TEAM-1");
    assert_eq!(detail.next_step, "none");
    assert!(detail.why.contains("carry the work"), "{}", detail.why);
    let landing = detail.landing.expect("a Landing Group section");
    assert_eq!(landing.label, "LANDING GROUP");
    let texts: Vec<&str> = landing.lines.iter().map(|l| l.text.as_str()).collect();
    // Members are spelled as `WorkItemId`'s `Display` spells every Pull
    // Request, as their own detail does (was `octo/app #7`, review of #49).
    assert_eq!(
        texts,
        [
            "octo/app#7 · open · Next Step Address review · Live Job · ambiguous, also attached to TEAM-2",
            "octo/api#8 · draft · Next Step Finish draft",
        ]
    );
    assert_eq!(landing.lines[0].job, Some(JobMarker::Live));
    assert!(landing.lines[0].caution, "ambiguous is a caution");
    assert_eq!(landing.lines[1].job, None);
    assert!(
        detail.also.iter().any(|a| a.contains("1 other attachment")),
        "{:?}",
        detail.also
    );
}

/// A Linear Issue with only a hint: its detail says no PR is attached and
/// names the hint as not a member; it stays Implement.
#[test]
fn a_hinted_linear_issue_detail_says_possibly_for_and_not_a_member() {
    let mut harness = harness();
    let detail = detail_of(&mut harness, "TEAM-3");
    assert_eq!(detail.next_step, "Implement");
    let texts: Vec<String> = detail
        .landing
        .expect("section")
        .lines
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert_eq!(
        texts,
        [
            "no Pull Request attached in Linear",
            // Spelled as `WorkItemId`'s `Display` (was `octo/api #9`).
            "possibly for TEAM-3: octo/api#9 (branch team-3-cache), not a member",
        ]
    );

    let pr = detail_of(&mut harness, "#9");
    let lines: Vec<String> = pr
        .landing
        .expect("section")
        .lines
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert!(lines[0].starts_with("possibly for TEAM-3"), "{lines:?}");
}

/// Attachments den didn't read completely (Linear returned none, errors
/// alongside the page, or more than den reads) never read as "no Pull
/// Request attached": with no member read, the section says den can't tell;
/// with some, it says more may be attached. Either way it agrees with the
/// Explanation ("couldn't establish whether a PR exists") and is a caution.
#[test]
fn a_linear_issue_detail_never_says_no_pr_when_attachments_are_incomplete() {
    let mut harness = harness_with(vec![
        linear_issue("octo", "id-4", "TEAM-4")
            .attachments_incomplete("Linear didn't return them")
            .build(),
        linear_issue("octo", "id-5", "TEAM-5")
            .attached("octo/api", 8)
            .attachments_incomplete("more than 50 attachments")
            .build(),
    ]);

    let none_read = detail_of(&mut harness, "TEAM-4");
    assert!(
        none_read
            .why
            .contains("couldn't establish whether a PR exists"),
        "{}",
        none_read.why
    );
    let landing = none_read.landing.expect("a Landing Group section");
    let texts: Vec<&str> = landing.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "den couldn't read all of its attachments in Linear (Linear didn't return them): it can't tell whether a Pull Request is attached"
        ]
    );
    assert!(landing.lines[0].caution, "unknown is a caution");

    let some_read = detail_of(&mut harness, "TEAM-5");
    let landing = some_read.landing.expect("a Landing Group section");
    let texts: Vec<&str> = landing.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            // Spelled as `WorkItemId`'s `Display` (was `octo/api #8`).
            "octo/api#8 · draft · Next Step Finish draft · ambiguous, also attached to TEAM-1",
            "den couldn't read all of its attachments in Linear (more than 50 attachments): more Pull Requests may be attached",
        ]
    );
    assert!(landing.lines[1].caution, "unknown is a caution");
}

/// A link that looks like a Pull Request den can't read
/// (`github.com/octo/api/pull/8`, no scheme) is no member, and the detail
/// never says "no Pull Request attached" for it: it names the link as a
/// caution, agreeing with the Explanation's unknown.
#[test]
fn a_linear_issue_detail_names_a_link_that_looks_like_a_pull_request() {
    let mut harness = harness_with(vec![
        linear_issue("octo", "id-6", "TEAM-6")
            .link("github.com/octo/api/pull/8")
            .build(),
    ]);
    let detail = detail_of(&mut harness, "TEAM-6");
    assert_eq!(detail.next_step, "none");
    assert!(
        detail
            .why
            .contains("couldn't establish whether a PR exists"),
        "{}",
        detail.why
    );
    let landing = detail.landing.expect("a Landing Group section");
    let texts: Vec<&str> = landing.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        ["attached github.com/octo/api/pull/8: looks like a Pull Request den can't read"]
    );
    assert!(landing.lines[0].caution, "unknown is a caution");
}

/// "Ordering and Scope": a Repo Scope shows the Linear Issues with a member
/// there (not a hint), and the Decisions shown are the ones the whole
/// survey gave.
#[test]
fn a_repo_scope_shows_the_linear_issues_with_a_member_there() {
    let mut harness = harness();
    let whole = harness.view.list.rows.clone();
    let shown_in = |harness: &mut Harness, repo: &str| -> Vec<String> {
        harness.key(Key::Tab);
        let target = Scope::Repo(RepoId::new("octo", repo));
        let index = harness
            .view
            .sidebar
            .rows
            .iter()
            .position(|r| r.scope == target)
            .unwrap();
        let current = harness.view.sidebar.cursor.unwrap_or(0);
        let key = if index >= current { Key::Down } else { Key::Up };
        for _ in 0..index.abs_diff(current) {
            harness.key(key);
        }
        // Enter scopes to the row and gives the list focus back.
        harness.key(Key::Enter);
        assert!(
            harness.view.header.scope.contains(&format!("octo/{repo}")),
            "{}",
            harness.view.header.scope
        );
        for row in &harness.view.list.rows {
            let same = whole.iter().find(|w| w.id == row.id).unwrap();
            assert_eq!(
                (row.next_step, &row.step_label, &row.waiting_label),
                (same.next_step, &same.step_label, &same.waiting_label),
                "narrowing changed {}",
                row.reference
            );
        }
        harness
            .view
            .list
            .rows
            .iter()
            .filter(|r| r.kind == WorkItemKind::LinearIssue)
            .map(|r| r.reference.clone())
            .collect()
    };
    assert_eq!(shown_in(&mut harness, "app"), ["TEAM-1", "TEAM-2"]);
    assert_eq!(shown_in(&mut harness, "api"), ["TEAM-1"]);
}

/// Acceptance: the group tag, "possibly for" and the ambiguity marker read
/// in monochrome, with Unicode and with ASCII glyphs.
#[test]
fn group_tags_read_in_monochrome() {
    let mut harness = harness();
    detail_of(&mut harness, "TEAM-1");
    let view = harness.view.clone();
    let (cols, rows) = SIZE;
    for (glyphs, mark) in [(GlyphProfile::Unicode, "◇"), (GlyphProfile::Ascii, "[L]")] {
        let theme = Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
        let text = frame::buffer_text(&frame::render(&theme, &view, cols, rows).expect("renders"));
        for expected in [
            format!("octo/app #7  {mark} TEAM-1 TEAM-2 ambiguous"),
            format!("octo/api #8  {mark} TEAM-1"),
            "octo/api #9  possibly for TEAM-3".to_owned(),
            format!("{mark} LANDING GROUP"),
            "ambiguous, also attached to TEAM-2".to_owned(),
        ] {
            assert!(
                text.contains(&expected),
                "{glyphs:?}: missing {expected:?}\n{text}"
            );
        }
    }
}

/// A member PR in a Held Repo reads in the Landing Group as it does in the
/// list and the detail: "held for release", never "no Next Step".
#[test]
fn a_held_member_reads_as_held_for_release_in_the_landing_group() {
    let mut workflow = linear_workflow("octo", "acme");
    workflow.held_repos = vec![RepoId::new("octo", "app")];
    let config = Config {
        owners: vec!["octo".to_owned()],
        main_clone_root: Some(PathBuf::from(ROOT)),
        main_clones: std::collections::BTreeMap::new(),
        linear: vec![workflow],
    };
    let mut harness = Harness::new(Ok(config));
    let (cols, rows) = SIZE;
    harness.send(Event::Resized { cols, rows });
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("octo")
        .repo("octo/app")
        // Ready, passing and approved at its head: it would Merge, but its
        // Repo is held.
        .pull_request(
            pr("octo/app", 7)
                .author(ME)
                .passing("ci")
                .approved()
                .review_at_head("reviewer", ReviewVerdict::Approved),
        )
        .build()));
    let mut coverage = Coverage::default();
    for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
        coverage.record(
            Source::Owner {
                owner: "octo".to_owned(),
                kind: SourceKind::Linear(kind),
            },
            SourceState::Complete,
        );
    }
    harness.linear.push_answer(LinearSurvey {
        part: LinearPart {
            viewers: vec![linear_viewer("octo", "acme")],
            issues: vec![
                linear_issue("octo", "id-1", "TEAM-1")
                    .attached("octo/app", 7)
                    .build(),
            ],
            ..LinearPart::default()
        },
        coverage,
    });
    harness.start();

    let pr_detail = detail_of(&mut harness, "#7");
    assert_eq!(pr_detail.next_step, "none (held for release)");
    let detail = detail_of(&mut harness, "TEAM-1");
    let landing = detail.landing.expect("a Landing Group section");
    let texts: Vec<&str> = landing.lines.iter().map(|l| l.text.as_str()).collect();
    // Was "octo/app #7 · …". Changed in the merge of #49 (634c48b), which
    // spells a member PR one way everywhere, `owner/repo#n`; the line's
    // content (open, held for release) is unchanged.
    assert_eq!(texts, ["octo/app#7 · open · held for release"]);
}
