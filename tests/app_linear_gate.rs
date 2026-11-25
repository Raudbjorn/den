//! Explicit Repo pairing is the boundary for every Linear view and action.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use den::{
    app::{App, Effect, Event, Key, SurveyId},
    cache::CachedObservation,
    domain::{
        Coverage, LinearPart, LinearSourceKind, NextStep, RepoId, Source, SourceKind, SourceState,
        WorkItemKind,
    },
    ports::LinearSurvey,
    start::Start,
};
use support::{
    build::{ME, ObservationBuilder, linear_issue, linear_viewer, now, pr},
    harness::{Harness, config, config_with_linear},
};

fn repo() -> RepoId {
    RepoId::new("acme", "api")
}
fn pair(app: &mut App, project: &str) -> Vec<Effect> {
    app.activate_linear_pairing(
        repo(),
        "workspace-id".into(),
        "acme".into(),
        project.into(),
        "user-me".into(),
        "den-connection:one".into(),
    )
}
fn linear_id(effects: &[Effect]) -> SurveyId {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SurveyLinear { id, .. } => Some(*id),
            _ => None,
        })
        .expect("Linear survey")
}
fn github(app: &mut App, effects: &[Effect]) {
    app.handle(Event::CacheLoaded { outcome: Ok(None) });
    let id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Survey { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    app.handle(Event::SurveyFinished {
        id,
        outcome: Ok(Box::new(
            ObservationBuilder::new()
                .fully_read_owner("acme")
                .repo("acme/api")
                .repo("acme/sibling")
                .build(),
        )),
        now: now(),
    });
}
fn survey() -> LinearSurvey {
    let mut coverage = Coverage::default();
    for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
        coverage.record(
            Source::Owner {
                owner: "acme".into(),
                kind: SourceKind::Linear(kind),
            },
            SourceState::Complete,
        );
    }
    LinearSurvey {
        part: LinearPart {
            viewers: vec![linear_viewer("acme", "acme")],
            issues: vec![linear_issue("acme", "issue-1", "TEAM-1").build()],
            ..Default::default()
        },
        coverage,
        ..Default::default()
    }
}
fn has_linear(app: &App) -> bool {
    app.view()
        .list
        .rows
        .iter()
        .any(|row| row.kind == WorkItemKind::LinearIssue)
}

#[test]
fn configured_owner_credentials_never_activate_an_unpaired_repo_or_owner_scope() {
    for selected in [false, true] {
        let mut app = App::new(Ok(config_with_linear(&["acme"], &[("acme", "acme")])));
        if selected {
            app = app.with_start(Start::Repo(repo()));
        }
        let (_, effects) = app.handle(Event::Started { now: now() });
        assert!(app.linear_binding().is_none());
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, Effect::SurveyLinear { .. }))
        );
        assert_eq!(
            app.view().footer.hints.iter().any(|hint| hint.key == "L"),
            selected
        );
        assert_eq!(app.configured_linear_env_names(), ["DEN_TEST_LINEAR_KEY"]);
    }
    let mut all = App::new(Ok(config(&["acme"])));
    assert!(pair(&mut all, "project-1").is_empty());
    assert!(
        all.linear_binding().is_none(),
        "an authenticated pairing alone cannot activate All Owners"
    );
}

#[test]
fn repo_without_workflow_defaults_can_pair_without_enabling_start_or_review_policy() {
    let mut app = App::new(Ok(config(&["acme"]))).with_start(Start::Repo(repo()));
    assert!(
        pair(&mut app, "project-1").is_empty(),
        "startup owns the initial read"
    );
    let (_, effects) = app.handle(Event::Started { now: now() });
    let workflow = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SurveyLinear { request, .. } => request.workflows.first(),
            _ => None,
        })
        .unwrap();
    assert_eq!(workflow.binding.as_ref().unwrap().project_id, "project-1");
    assert!(workflow.policy().review.is_none());
    assert!(workflow.start_teams.is_empty());
    assert!(workflow.in_progress_state.is_none());
    assert!(workflow.release_state.is_none());
    assert_eq!(workflow.new_work_repos, [repo()]);
    github(&mut app, &effects);
    app.handle(Event::LinearSurveyFinished {
        id: linear_id(&effects),
        survey: Box::new(survey()),
        now: now(),
    });
    assert!(
        has_linear(&app),
        "assigned project issues belong to the selected Repo even without an attached PR"
    );
    assert_eq!(
        app.view()
            .sidebar
            .rows
            .iter()
            .find(|row| row.current)
            .unwrap()
            .count,
        1
    );
}

#[test]
fn switching_scope_revokes_pairing_and_ignores_a_late_linear_response() {
    let mut app = App::new(Ok(config(&["acme"]))).with_start(Start::Repo(repo()));
    pair(&mut app, "project-1");
    let (_, effects) = app.handle(Event::Started { now: now() });
    github(&mut app, &effects);
    app.handle(Event::Key(Key::Left));
    assert!(app.selected_repo().is_none());
    assert!(app.linear_binding().is_none());
    app.handle(Event::LinearSurveyFinished {
        id: linear_id(&effects),
        survey: Box::new(survey()),
        now: now(),
    });
    assert!(!has_linear(&app));
}

#[test]
fn replacing_a_project_clears_data_and_old_authentication_failures_cannot_revoke_it() {
    let mut app = App::new(Ok(config(&["acme"]))).with_start(Start::Repo(repo()));
    pair(&mut app, "project-1");
    let (_, effects) = app.handle(Event::Started { now: now() });
    github(&mut app, &effects);
    let old = linear_id(&effects);
    app.handle(Event::LinearSurveyFinished {
        id: old,
        survey: Box::new(survey()),
        now: now(),
    });
    assert!(has_linear(&app));
    let fresh = pair(&mut app, "project-2");
    assert_ne!(old, linear_id(&fresh));
    assert!(!has_linear(&app));
    app.handle(Event::LinearSurveyFinished {
        id: old,
        survey: Box::new(LinearSurvey {
            authentication_failed: true,
            ..Default::default()
        }),
        now: now(),
    });
    assert_eq!(app.linear_binding().unwrap().project_id, "project-2");
    app.handle(Event::LinearSurveyFinished {
        id: linear_id(&fresh),
        survey: Box::new(survey()),
        now: now(),
    });
    assert!(has_linear(&app));
}

#[test]
fn cache_identity_must_match_every_partition_and_legacy_cache_is_not_trusted() {
    let scope = [
        "acme",
        "api",
        "workspace-id",
        "project-1",
        "user-me",
        "den-connection:one",
    ];
    let mut scopes = vec![None];
    for index in 0..scope.len() {
        let mut mismatch: Vec<String> = scope.iter().map(|s| (*s).to_owned()).collect();
        mismatch[index] = "different".into();
        scopes.push(Some(mismatch));
    }
    scopes.push(Some(scope.iter().map(|s| (*s).to_owned()).collect()));
    let last = scopes.len() - 1;
    for (index, saved_scope) in scopes.into_iter().enumerate() {
        let mut app = App::new(Ok(config(&["acme"]))).with_start(Start::Repo(repo()));
        pair(&mut app, "project-1");
        app.handle(Event::Started { now: now() });
        let mut observation = ObservationBuilder::new()
            .fully_read_owner("acme")
            .repo("acme/api")
            .build();
        let linear = survey();
        observation.linear = linear.part;
        for entry in linear.coverage.entries() {
            observation
                .coverage
                .record(entry.source.clone(), entry.state.clone());
        }
        let mut cached = CachedObservation::fresh(observation);
        cached.linear_scope = saved_scope;
        app.handle(Event::CacheLoaded {
            outcome: Ok(Some(Box::new(cached))),
        });
        assert_eq!(has_linear(&app), index == last, "partition {index}");
    }
}

#[test]
fn auth_loss_revokes_but_network_failure_keeps_binding_and_stale_data() {
    for authentication_failed in [false, true] {
        let mut app = App::new(Ok(config(&["acme"]))).with_start(Start::Repo(repo()));
        pair(&mut app, "project-1");
        let (_, effects) = app.handle(Event::Started { now: now() });
        github(&mut app, &effects);
        app.handle(Event::LinearSurveyFinished {
            id: linear_id(&effects),
            survey: Box::new(survey()),
            now: now(),
        });
        let (_, refresh) = app.handle(Event::Key(Key::Char('r')));
        let mut failed = survey();
        failed.part = LinearPart::default();
        failed.authentication_failed = authentication_failed;
        for kind in [LinearSourceKind::Viewer, LinearSourceKind::AssignedIssues] {
            failed.coverage.record(
                Source::Owner {
                    owner: "acme".into(),
                    kind: SourceKind::Linear(kind),
                },
                SourceState::Failed {
                    error: "unavailable".into(),
                },
            );
        }
        app.handle(Event::LinearSurveyFinished {
            id: linear_id(&refresh),
            survey: Box::new(failed),
            now: now(),
        });
        assert_eq!(app.linear_binding().is_some(), !authentication_failed);
        assert_eq!(has_linear(&app), !authentication_failed);
    }
}

#[test]
fn landing_a_group_with_an_action_in_another_repo_is_blocked_as_a_whole() {
    let mut harness = Harness::new(Ok(config_with_linear(&["acme"], &[("acme", "acme")])))
        .with_linear_pairing("acme/api", "acme");
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .repo("acme/sibling")
        .pull_request(pr("acme/api", 1).author(ME).passing("ci").approved())
        .pull_request(pr("acme/sibling", 2).author(ME).passing("ci").approved())
        .build()));
    let mut answer = survey();
    answer.part.issues = vec![
        linear_issue("acme", "issue-1", "TEAM-1")
            .attached("acme/api", 1)
            .attached("acme/sibling", 2)
            .build(),
    ];
    harness.linear.push_answer(answer);
    harness.start();
    let issue = harness
        .view
        .list
        .rows
        .iter()
        .find(|row| row.kind == WorkItemKind::LinearIssue)
        .unwrap();
    assert_ne!(issue.next_step, Some(NextStep::Land));
    assert!(
        harness
            .effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Launch { .. }))
    );
}

#[test]
fn an_unpaired_repo_cannot_reopen_a_historical_linear_group_through_its_pr() {
    use den::{
        app::view_model::{LaunchStatus, Overlay},
        domain::{LinearIssueId, WorkItemId, facts::PullRequestRef},
        jobs::{JobRecord, JobSubject, LandedMember, LandingGroupSubject},
    };
    const CLONE: &str = "/work/main";
    const TREE: &str = "/work/main/.claude/worktrees/old-land";
    const HEAD: &str = "0000000000000000000000000000000000000001";
    let mut settings = config(&["acme"]);
    settings.main_clones.insert("acme/api".into(), CLONE.into());
    let mut harness = Harness::new(Ok(settings)).with_start(Start::Repo(repo()));
    harness
        .workspace
        .add_clone(CLONE, &[("origin", "git@github.com:acme/api.git")]);
    harness
        .workspace
        .set_remote_head(CLONE, "origin", "feature-1", HEAD);
    harness.workspace.add_existing(TREE);
    harness
        .workspace
        .register_worktree(CLONE, TREE, Some("old-land"));
    harness.workspace.set_local_branch(CLONE, "old-land", HEAD);
    harness.jobs.holding(JobRecord {
        subject: JobSubject::LandingGroup(LandingGroupSubject {
            issue: WorkItemId::LinearIssue {
                owner: "acme".into(),
                issue: LinearIssueId::new("acme", "old-issue"),
            },
            identifier: "TEAM-OLD".into(),
            members: vec![LandedMember {
                pr: PullRequestRef::new(repo(), 1),
                head: HEAD.into(),
            }],
        }),
        next_step: NextStep::Land,
        target: repo(),
        main_clone: CLONE.into(),
        worktree: TREE.into(),
        branch: "old-land".into(),
        session: "old-land-session".into(),
        launched_at: now(),
        permission_mode: None,
    });
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .pull_request(pr("acme/api", 1).author(ME).changes_requested())
        .build()));
    harness.start();
    assert!(
        harness.view.list.rows.iter().all(|row| row.job.is_none()),
        "the old Linear Job is unavailable while unpaired"
    );
    harness.settle(Event::Key(Key::Char('c')));
    let Some(Overlay::LaunchSheet(sheet)) = &harness.view.overlay else {
        panic!("ordinary GitHub launch remains available")
    };
    assert_ne!(sheet.status, LaunchStatus::LiveJob);
    harness.settle(Event::Key(Key::Char('o')));
    assert!(
        harness
            .effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Reopen { .. }))
    );
}

#[test]
fn discarding_a_linear_cache_partition_also_discards_its_member_state_evidence() {
    use den::domain::facts::PullRequestState;
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .member_state("acme/api", 1, PullRequestState::Merged { merged_at: None })
        .build();
    let discarded = CachedObservation::fresh(observation).restricted_to_linear(&[]);
    assert!(discarded.observation.member_states.is_empty());
    assert!(discarded.observation.member_attempts.is_empty());
    assert!(
        discarded
            .observation
            .coverage
            .entries()
            .iter()
            .all(|entry| !entry.source.is_member_states())
    );
}

#[test]
fn workflow_review_and_group_policy_never_changes_an_unpaired_sibling_pr() {
    use den::{decide::decide, domain::Viewers};
    let observation = ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .repo("acme/sibling")
        .pull_request(pr("acme/api", 1).author(ME).passing("ci").approved())
        .pull_request(pr("acme/sibling", 2).author(ME).passing("ci").approved())
        .linear_viewer("acme", "acme")
        .linear_issue(
            linear_issue("acme", "issue-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/sibling", 2),
        )
        .build();
    let policies = config_with_linear(&["acme"], &[("acme", "acme")])
        .policies()
        .with_linear_repo(repo());
    let decisions = decide(&observation, &Viewers::of(&observation), &policies, now());
    let selected = decisions
        .iter()
        .find(|decision| decision.item.id.repo() == Some(&repo()))
        .unwrap();
    assert_eq!(selected.next_step, Some(NextStep::RequestReview));
    let sibling = decisions
        .iter()
        .find(|decision| {
            decision
                .item
                .id
                .repo()
                .is_some_and(|repo| repo.name == "sibling")
        })
        .unwrap();
    assert_eq!(sibling.next_step, Some(NextStep::Merge));
    assert!(sibling.suggested_reviewers.is_none());
    assert!(sibling.landing.groups.is_empty());
    assert!(sibling.held_back.is_none());
}
