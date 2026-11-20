//! Seam 1, Landing Groups (#35): attachments make members, branch names only
//! hint; a Pull Request attached to two Linear Issues is ambiguous in both; a
//! member den can't see is "not surveyed"; a Linear Issue with open members
//! has no Next Step of its own; incomplete attachments withhold Implement;
//! Decisions are made on the whole Observation, so Scope narrowing changes
//! none of them; Pull Requests are Owner + Repo + number, Linear Issues
//! workspace + permanent ID.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{
        Decision,
        correlation::{MemberStatus, NotSurveyed},
        decide,
    },
    domain::{
        Incompleteness, NextStep, Observation, Policies, RepoId, SourceKind, SourceState, Viewers,
        WaitingOn, WorkItemId, facts::PullRequestRef,
    },
    ordering::{Scope, in_scope, same_work_item},
};
use support::build::{ME, ObservationBuilder, linear_issue, now, pr};

/// What den shows as the reason for withholding Start and Implement.
const PR_EXISTENCE_UNKNOWN: &str = "couldn't establish whether a PR exists";

fn decide_all(observation: &Observation) -> Vec<Decision> {
    decide(
        observation,
        &Viewers::of(observation),
        &Policies::default(),
        now(),
    )
}

fn linear<'d>(decisions: &'d [Decision], identifier: &str) -> &'d Decision {
    decisions
        .iter()
        .find(|d| d.item.identifier.as_deref() == Some(identifier))
        .unwrap_or_else(|| panic!("no Decision for {identifier}: {decisions:#?}"))
}

fn pull_request<'d>(decisions: &'d [Decision], repo: &str, number: u64) -> &'d Decision {
    let id = pr_id(repo, number);
    decisions
        .iter()
        .find(|d| same_work_item(&d.item.id, &id))
        .unwrap_or_else(|| panic!("no Decision for {repo}#{number}: {decisions:#?}"))
}

fn pr_id(repo: &str, number: u64) -> WorkItemId {
    let (owner, name) = repo.split_once('/').unwrap();
    WorkItemId::PullRequest {
        repo: RepoId::new(owner, name),
        number,
    }
}

fn pr_ref(repo: &str, number: u64) -> PullRequestRef {
    let (owner, name) = repo.split_once('/').unwrap();
    PullRequestRef::new(RepoId::new(owner, name), number)
}

fn identifiers(groups: &[den::decide::correlation::GroupRef]) -> Vec<&str> {
    groups.iter().map(|g| g.identifier.as_str()).collect()
}

/// acme (Linear Workflow) and octo, each with `app` and `api` Repos, every
/// source read completely.
fn two_owners() -> ObservationBuilder {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/app")
        .repo("acme/api")
        .fully_read_owner("octo")
        .repo("octo/app")
        .linear_viewer("acme", "acme")
}

/// Story 22: two Owners' `app#1` are different Pull Requests. Only the
/// attached one is a member, and only it names the group; Owner and Repo
/// compare as GitHub's do, case-insensitively.
#[test]
fn the_same_repo_name_and_pr_number_under_two_owners_dont_collide() {
    let observation = two_owners()
        .pull_request(pr("acme/app", 1).author(ME))
        .pull_request(pr("octo/app", 1).author(ME))
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("ACME/App", 1))
        .build();
    let decisions = decide_all(&observation);

    let group = &linear(&decisions, "TEAM-1").landing;
    assert_eq!(group.members.len(), 1, "{group:?}");
    assert_eq!(group.members[0].status, MemberStatus::Open { draft: false });
    assert_eq!(
        identifiers(&pull_request(&decisions, "acme/app", 1).landing.groups),
        ["TEAM-1"]
    );
    assert!(
        pull_request(&decisions, "octo/app", 1).landing.is_empty(),
        "octo's app#1 is no member"
    );
}

/// Story 24: `TEAM-1` matches its identifier on word boundaries only, in
/// any case; `team-12-…` and `steam-1` are not hints for it.
#[test]
fn team_1_does_not_match_a_team_12_branch() {
    let observation = two_owners()
        .pull_request(pr("acme/api", 1).author(ME).head_ref("team-12-cache"))
        .pull_request(pr("acme/api", 2).author(ME).head_ref("me/TEAM-1-fix"))
        .pull_request(pr("acme/api", 3).author(ME).head_ref("steam-1"))
        .pull_request(pr("acme/api", 4).author(ME).head_ref("team-1"))
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
        .linear_issue(linear_issue("acme", "id-12", "TEAM-12"))
        .build();
    let decisions = decide_all(&observation);

    let hinted: Vec<PullRequestRef> = linear(&decisions, "TEAM-1")
        .landing
        .hints
        .iter()
        .map(|h| h.pr.clone())
        .collect();
    assert_eq!(hinted, [pr_ref("acme/api", 2), pr_ref("acme/api", 4)]);
    assert_eq!(
        identifiers(&pull_request(&decisions, "acme/api", 1).landing.possibly_for),
        ["TEAM-12"]
    );
    assert!(pull_request(&decisions, "acme/api", 3).landing.is_empty());
}

/// Story 23: a hint is shown as "possibly for", and that is all it does: it
/// makes no member, doesn't withhold Implement, and leaves the Pull
/// Request's own Decision as it was. Another Owner's branch is no hint.
#[test]
fn a_hint_never_makes_a_member_or_withholds_implement() {
    let github = || {
        two_owners()
            .pull_request(
                pr("acme/api", 4)
                    .author(ME)
                    .head_ref("team-1-fix")
                    .approved()
                    .passing("ci"),
            )
            .pull_request(pr("octo/app", 5).author(ME).head_ref("team-1-fix"))
    };
    let without = decide_all(&github().build());
    let decisions = decide_all(
        &github()
            .linear_issue(linear_issue("acme", "id-1", "TEAM-1"))
            .build(),
    );

    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, Some(NextStep::Implement), "{issue:?}");
    assert!(issue.landing.members.is_empty());
    assert_eq!(
        issue.landing.hints.len(),
        1,
        "octo's branch is no hint: {issue:?}"
    );
    assert!(
        issue
            .explanation
            .other_conditions
            .iter()
            .any(|c| c.contains("possibly for TEAM-1") && c.contains("acme/api #4")),
        "{issue:?}"
    );

    let hinted = pull_request(&decisions, "acme/api", 4);
    assert!(hinted.landing.groups.is_empty());
    assert_eq!(identifiers(&hinted.landing.possibly_for), ["TEAM-1"]);
    let before = pull_request(&without, "acme/api", 4);
    assert_eq!(hinted.next_step, Some(NextStep::Merge));
    assert_eq!(
        (hinted.next_step, &hinted.waiting_on, &hinted.explanation),
        (before.next_step, &before.waiting_on, &before.explanation)
    );
}

/// Story 25: one Pull Request attached to two Linear Issues is a member of
/// both, and ambiguous in both and on its own row.
#[test]
fn one_pr_attached_to_two_issues_is_ambiguous_in_both() {
    let observation = two_owners()
        .pull_request(pr("acme/api", 4).author(ME))
        .pull_request(pr("acme/app", 5).author(ME))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 4)
                .attached("acme/app", 5),
        )
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2").attached("acme/api", 4))
        .build();
    let decisions = decide_all(&observation);

    for (identifier, other) in [("TEAM-1", "TEAM-2"), ("TEAM-2", "TEAM-1")] {
        let decision = linear(&decisions, identifier);
        let member = decision
            .landing
            .members
            .iter()
            .find(|m| m.pr == pr_ref("acme/api", 4))
            .unwrap_or_else(|| panic!("{identifier} has acme/api #4: {decision:?}"));
        assert!(member.ambiguous());
        assert_eq!(identifiers(&member.also_attached_to), [other]);
        assert!(
            decision
                .explanation
                .other_conditions
                .iter()
                .any(|c| c.contains("acme/api #4") && c.contains("ambiguous")),
            "{decision:?}"
        );
    }
    let not_shared = linear(&decisions, "TEAM-1")
        .landing
        .members
        .iter()
        .find(|m| m.pr == pr_ref("acme/app", 5))
        .unwrap();
    assert!(!not_shared.ambiguous());

    let shared = pull_request(&decisions, "acme/api", 4);
    assert!(shared.landing.ambiguous());
    assert_eq!(identifiers(&shared.landing.groups), ["TEAM-1", "TEAM-2"]);
    assert!(!pull_request(&decisions, "acme/app", 5).landing.ambiguous());
}

/// Story 27: a member den can't see is "member not surveyed", and then den
/// can't tell whether a Pull Request carries the work: no Implement. That
/// holds for a Repo den didn't survey, another Owner's Repo (surveyed or
/// not: a Landing Group's Pull Requests are its Owner's), and a Repo whose
/// open Pull Requests den didn't read completely.
#[test]
fn a_member_den_did_not_survey_is_not_surveyed_and_withholds_implement() {
    let observation = two_owners()
        .repo("acme/web")
        .repo_coverage(
            "acme/web",
            SourceKind::PullRequests,
            SourceState::Failed {
                error: "timed out".to_owned(),
            },
        )
        .pull_request(pr("octo/app", 1).author(ME))
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("acme/ghost", 3))
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2").attached("octo/app", 1))
        .linear_issue(linear_issue("acme", "id-3", "TEAM-3").attached("acme/web", 9))
        .build();
    let decisions = decide_all(&observation);

    for (identifier, why) in [
        ("TEAM-1", NotSurveyed::RepoNotSurveyed),
        ("TEAM-2", NotSurveyed::OtherOwner),
        ("TEAM-3", NotSurveyed::PullRequestsIncomplete),
    ] {
        let decision = linear(&decisions, identifier);
        assert_eq!(
            decision.landing.members[0].status,
            MemberStatus::NotSurveyed(why),
            "{identifier}"
        );
        assert_eq!(decision.next_step, None, "{decision:?}");
        let reason = decision.explanation.no_step_reason.as_deref().unwrap();
        assert!(reason.contains(PR_EXISTENCE_UNKNOWN), "{reason}");
        assert!(reason.contains("member not surveyed"), "{reason}");
        assert!(
            decision
                .explanation
                .unknowns
                .iter()
                .any(|u| u.contains(why.describe())),
            "{decision:?}"
        );
    }
}

/// Story 30: a Linear Issue with an open member has no Next Step of its
/// own; its Pull Requests carry the work, each with its own Next Step.
#[test]
fn a_linear_issue_with_open_members_has_no_next_step_of_its_own() {
    let observation = two_owners()
        .pull_request(pr("acme/api", 4).author(ME).draft())
        .pull_request(pr("acme/app", 5).author(ME).conflicting())
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 4)
                .attached("acme/app", 5)
                .link("https://www.figma.com/file/abc"),
        )
        .build();
    let decisions = decide_all(&observation);

    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:?}");
    assert_eq!(issue.waiting_on, WaitingOn::Nobody);
    assert!(!issue.unattributed);
    let reason = issue.explanation.no_step_reason.as_deref().unwrap();
    assert!(
        reason.contains("carry the work")
            && reason.contains("acme/api #4")
            && reason.contains("acme/app #5"),
        "{reason}"
    );
    assert!(issue.explanation.unknowns.is_empty(), "{issue:?}");
    assert!(
        issue
            .explanation
            .other_conditions
            .iter()
            .any(|c| c.contains("1 other attachment")),
        "{issue:?}"
    );
    assert_eq!(
        issue.landing.members[0].status,
        MemberStatus::Open { draft: true }
    );
    assert_eq!(
        pull_request(&decisions, "acme/api", 4).next_step,
        Some(NextStep::FinishDraft)
    );
    assert_eq!(
        pull_request(&decisions, "acme/app", 5).next_step,
        Some(NextStep::ResolveConflicts)
    );
}

/// An attached Pull Request den looked for in a completely read Repo and
/// didn't find isn't open: merged or closed. The issue then has members but
/// none open, so it isn't Implement either (Mark for release is #37's).
#[test]
fn a_member_missing_from_a_fully_read_repo_is_merged_or_closed() {
    let observation = two_owners()
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("acme/api", 4))
        .build();
    let decisions = decide_all(&observation);
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.landing.members[0].status, MemberStatus::NotOpen);
    assert_eq!(issue.next_step, None);
    assert!(
        issue
            .explanation
            .no_step_reason
            .as_deref()
            .is_some_and(|r| r.contains("merged or closed")),
        "{issue:?}"
    );
}

/// A Repo with its own Pull Request source is judged by that source alone,
/// as the cache merge judges it (`cache/merge.rs`, `Reading`): read
/// completely, a member missing from it is merged or closed even when a
/// later page of its Owner's Repo list was lost. A Repo without a source of
/// its own was read as part of its Owner's list, so the Owner's partial
/// list leaves its members not surveyed.
#[test]
fn a_repo_read_completely_on_its_own_decides_its_members_whatever_its_owner() {
    let observation = two_owners()
        .owner_coverage(
            "acme",
            SourceKind::PullRequests,
            SourceState::Partial(Incompleteness::PaginationStopped {
                pages_read: 2,
                reason: "rate limited".to_owned(),
            }),
        )
        .repo_coverage("acme/api", SourceKind::PullRequests, SourceState::Complete)
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("acme/api", 4))
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2").attached("acme/app", 5))
        .build();
    let decisions = decide_all(&observation);
    assert_eq!(
        linear(&decisions, "TEAM-1").landing.members[0].status,
        MemberStatus::NotOpen
    );
    assert_eq!(
        linear(&decisions, "TEAM-2").landing.members[0].status,
        MemberStatus::NotSurveyed(NotSurveyed::PullRequestsIncomplete)
    );
}

/// A Repo Scope is part of its Owner's Scope, so whatever a Repo Scope
/// shows, its Owner's shows too. A Linear Issue's group holds its own
/// Owner's Pull Requests only (CONTEXT.md, "Landing Group"): an attachment
/// in another Owner's Repo is "not surveyed" and doesn't put the issue in
/// that Repo's Scope.
#[test]
fn a_repo_scope_never_shows_what_its_owner_scope_hides() {
    let observation = two_owners()
        .pull_request(pr("acme/app", 1).author(ME))
        .pull_request(pr("octo/app", 1).author(ME))
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("octo/app", 1))
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2").attached("acme/app", 1))
        .build();
    let decisions = decide_all(&observation);
    let repos: Vec<RepoId> = observation.repos.iter().map(|r| r.id.clone()).collect();
    for repo in &repos {
        let repo_scope = Scope::Repo(repo.clone());
        let owner_scope = Scope::Owner(repo.owner.clone());
        for decision in &decisions {
            assert!(
                !repo_scope.contains_decision(decision) || owner_scope.contains_decision(decision),
                "{repo_scope} shows {} but {owner_scope} doesn't",
                decision.item.reference()
            );
        }
    }
    let other_owners = Scope::Repo(RepoId::new("octo", "app"));
    assert!(!other_owners.contains_decision(linear(&decisions, "TEAM-1")));
    let own = Scope::Repo(RepoId::new("acme", "app"));
    assert!(own.contains_decision(linear(&decisions, "TEAM-2")));
}

/// Story 16 with a group: the issue moves to another team (new
/// identifier, same permanent ID); it is the same Work Item with the same
/// members, and its Pull Request names the same Linear Issue.
#[test]
fn an_issue_moved_between_teams_keeps_its_identity_and_its_group() {
    let with = |identifier: &str| {
        decide_all(
            &two_owners()
                .pull_request(pr("acme/api", 4).author(ME))
                .linear_issue(linear_issue("acme", "id-1", identifier).attached("acme/api", 4))
                .build(),
        )
    };
    let before = with("TEAM-1");
    let after = with("OPS-7");
    let (was, is) = (linear(&before, "TEAM-1"), linear(&after, "OPS-7"));
    assert_eq!(was.item.id, is.item.id);
    assert_eq!(was.landing.members, is.landing.members);
    assert_eq!(was.next_step, is.next_step);

    let (pr_was, pr_is) = (
        &pull_request(&before, "acme/api", 4).landing.groups[0],
        &pull_request(&after, "acme/api", 4).landing.groups[0],
    );
    assert_eq!(pr_was.issue, pr_is.issue);
    assert_eq!(pr_is.issue, is.item.id);
    assert_eq!(pr_is.identifier, "OPS-7");
}

/// Story 26 and "Ordering and Scope": a Repo Scope includes the Linear
/// Issues with a member there (not a hint), and narrowing changes no
/// Decision: a group with a member elsewhere is decided the same in every
/// Scope.
#[test]
fn scope_narrowing_changes_no_decision() {
    let observation = two_owners()
        .repo("acme/web")
        .pull_request(pr("acme/api", 4).author(ME))
        .pull_request(pr("acme/web", 6).author(ME).head_ref("team-2-ui"))
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("acme/api", 4))
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2"))
        .build();
    let decisions = decide_all(&observation);
    let in_repo = |repo: &str| {
        let (owner, name) = repo.split_once('/').unwrap();
        Scope::Repo(RepoId::new(owner, name))
    };

    for scope in [
        Scope::AllOwners,
        Scope::Owner("acme".to_owned()),
        Scope::Owner("octo".to_owned()),
        in_repo("acme/api"),
        in_repo("ACME/web"),
        in_repo("acme/app"),
    ] {
        for shown in in_scope(&decisions, &scope) {
            let whole = decisions
                .iter()
                .find(|d| d.item.id == shown.item.id)
                .unwrap();
            assert_eq!(shown, whole, "{scope}");
        }
    }
    let shown = |scope: Scope| -> Vec<String> {
        in_scope(&decisions, &scope)
            .map(|d| d.item.reference())
            .collect()
    };
    assert!(shown(in_repo("acme/api")).contains(&"TEAM-1".to_owned()));
    assert!(!shown(in_repo("acme/app")).contains(&"TEAM-1".to_owned()));
    assert!(
        !shown(in_repo("acme/web")).contains(&"TEAM-2".to_owned()),
        "a hint doesn't bring a Linear Issue into a Repo Scope"
    );
    assert!(shown(Scope::Owner("acme".to_owned())).contains(&"TEAM-2".to_owned()));
}

/// Spec 0002, rule 4: when den couldn't read all of a Linear Issue's
/// attachments, it can't say there is no Pull Request: no Implement (and no
/// Start, which #33 adds under the same gate), with the unknown said. The
/// issue stays visible.
#[test]
fn partial_attachment_coverage_withholds_implement() {
    let observation = two_owners()
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attachments_incomplete("more than 50 attachments"),
        )
        .linear_issue(
            linear_issue("acme", "id-2", "TEAM-2")
                .unassigned()
                .attachments_incomplete("Linear returned errors alongside the page"),
        )
        .build();
    let decisions = decide_all(&observation);
    for identifier in ["TEAM-1", "TEAM-2"] {
        let decision = linear(&decisions, identifier);
        assert_eq!(decision.next_step, None, "{decision:?}");
        assert!(
            decision
                .explanation
                .no_step_reason
                .as_deref()
                .is_some_and(|r| r.contains(PR_EXISTENCE_UNKNOWN)),
            "{decision:?}"
        );
        assert!(
            decision
                .explanation
                .unknowns
                .iter()
                .any(|u| u.contains(PR_EXISTENCE_UNKNOWN)),
            "{decision:?}"
        );
    }
    assert!(
        linear(&decisions, "TEAM-1").explanation.unknowns[0].contains("more than 50"),
        "the reason is said"
    );
}

/// With every attachment read and none a Pull Request, an assigned Linear
/// Issue is Implement, and says so; links don't count as members.
#[test]
fn an_issue_with_only_links_attached_is_implement() {
    let observation = two_owners()
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1").link("https://github.com/acme/api/issues/3"),
        )
        .build();
    let decisions = decide_all(&observation);
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, Some(NextStep::Implement));
    assert!(issue.explanation.unknowns.is_empty(), "{issue:?}");
    assert!(issue.landing.members.is_empty());
}
