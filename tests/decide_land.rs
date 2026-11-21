//! Seam 1, Land and Mark for release (#37): the group gate and release
//! reconciliation. A member PR that would Merge "lands with" its Linear
//! Issue; the Linear Issue shows Land when every open member outside a Held
//! Repo would Merge, none is ambiguous, not surveyed or queued, and every
//! attachment and member fact was read; otherwise it names each blocking
//! member. With no open members, at least one member read merged, and a state
//! that is neither the release state nor completed or canceled, it shows Mark
//! for release; in the release state, release is outside den.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    decide::{Decision, HeldBack, correlation::MemberStatus, decide},
    domain::{
        Incompleteness, LinearSourceKind, LinearWorkflowPolicy, NextStep, Observation, Policies,
        RepoId, ReviewPolicy, Source, SourceKind, SourceState, Viewers, WaitingOn, WorkItemId,
        facts::{LinearStateKind, PullRequestState, ReviewVerdict},
    },
    ordering::same_work_item,
};
use support::build::{ME, ObservationBuilder, PrBuilder, linear_issue, now, pr, ts};

const RELEASE: &str = "Ready for Release";

/// acme's Linear Workflow: review rules with both switches on, Held Repo
/// `acme/web`, release state "Ready for Release".
fn acme() -> Policies {
    Policies::default().with(
        "acme",
        LinearWorkflowPolicy {
            review: Some(ReviewPolicy {
                exact_head_approval: true,
                comment_reviews_need_rereview: true,
                pending_requests_hold_merge: true,
                reviewers: Vec::new(),
            }),
            held_repos: vec![RepoId::new("acme", "web")],
            release_state: Some(RELEASE.to_owned()),
            ..LinearWorkflowPolicy::default()
        },
    )
}

/// acme (Linear Workflow) with `api`, `app` and the Held Repo `web`, every
/// source read completely, the Linear viewer read.
fn acme_owner() -> ObservationBuilder {
    ObservationBuilder::new()
        .fully_read_owner("acme")
        .repo("acme/api")
        .repo("acme/app")
        .repo("acme/web")
        .linear_viewer("acme", "acme")
}

/// A PR of mine that would Merge under acme's workflow: passing, mergeable,
/// approved by a user at its current head.
fn ready(repo: &str, number: u64) -> PrBuilder {
    pr(repo, number)
        .author(ME)
        .passing("ci")
        .approved()
        .review_at_head("alice", ReviewVerdict::Approved)
}

fn decide_all(observation: &Observation, policies: &Policies) -> Vec<Decision> {
    decide(observation, &Viewers::of(observation), policies, now())
}

fn linear<'d>(decisions: &'d [Decision], identifier: &str) -> &'d Decision {
    decisions
        .iter()
        .find(|d| d.item.identifier.as_deref() == Some(identifier))
        .unwrap_or_else(|| panic!("no Decision for {identifier}: {decisions:#?}"))
}

fn pull_request<'d>(decisions: &'d [Decision], repo: &str, number: u64) -> &'d Decision {
    let (owner, name) = repo.split_once('/').unwrap();
    let id = WorkItemId::PullRequest {
        repo: RepoId::new(owner, name),
        number,
    };
    decisions
        .iter()
        .find(|d| same_work_item(&d.item.id, &id))
        .unwrap_or_else(|| panic!("no Decision for {repo}#{number}: {decisions:#?}"))
}

/// Everything the Explanation says, in one string.
fn said(decision: &Decision) -> String {
    let e = &decision.explanation;
    format!(
        "{} | {} | {} | {}",
        e.winning_rule.as_deref().unwrap_or_default(),
        e.no_step_reason.as_deref().unwrap_or_default(),
        e.other_conditions.join(" | "),
        e.unknowns.join(" | ")
    )
}

fn assert_says(decision: &Decision, needles: &[&str]) {
    let text = said(decision);
    for needle in needles {
        assert!(text.contains(needle), "{needle:?} not in {text:?}");
    }
}

fn lands_with(identifiers: &[&str]) -> Option<HeldBack> {
    Some(HeldBack::LandsWith {
        identifiers: identifiers.iter().map(|i| (*i).to_owned()).collect(),
    })
}

/// Acceptance: all non-held members ready gives Land, Waiting On the Linear
/// assignee; each member shows "lands with", with no Next Step of its own.
#[test]
fn all_non_held_members_ready_gives_land_and_each_member_lands_with() {
    let observation = acme_owner()
        .pull_request(ready("acme/api", 1))
        .pull_request(ready("acme/app", 2))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_all(&observation, &acme());

    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, Some(NextStep::Land), "{issue:#?}");
    assert_eq!(issue.waiting_on, WaitingOn::Me, "the Linear assignee is me");
    assert!(
        issue.waiting_reason.contains("the Linear assignee"),
        "{issue:?}"
    );
    assert_says(
        issue,
        &[
            "every open member outside a Held Repo would Merge",
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "acme/api#1, acme/app#2",
        ],
    );
    for (repo, number) in [("acme/api", 1), ("acme/app", 2)] {
        let member = pull_request(&decisions, repo, number);
        assert_eq!(member.next_step, None, "{member:#?}");
        assert_eq!(member.held_back, lands_with(&["TEAM-1"]));
        assert_eq!(
            member.held_back.as_ref().unwrap().label(),
            "lands with TEAM-1"
        );
        assert_eq!(member.waiting_on, WaitingOn::Nobody);
        assert_says(member, &["lands with TEAM-1", "it would otherwise Merge"]);
    }
}

/// Acceptance: one member conflicting gives no Land, and the Explanation
/// names that member with its own Next Step. The ready member still lands
/// with the issue: it never merges by itself.
#[test]
fn one_member_conflicting_gives_no_land_and_names_the_member() {
    let observation = acme_owner()
        .pull_request(ready("acme/api", 1))
        .pull_request(ready("acme/app", 2).conflicting())
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_all(&observation, &acme());

    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_eq!(issue.waiting_on, WaitingOn::Nobody);
    assert_says(
        issue,
        &[
            "carry the work",
            "1 blocking",
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "blocking: acme/app#2: its own Next Step is Resolve conflicts (conflicts with main)",
        ],
    );
    assert!(
        // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
        !said(issue).contains("blocking: acme/api#1"),
        "the ready member doesn't block: {}",
        said(issue)
    );
    assert_eq!(
        pull_request(&decisions, "acme/app", 2).next_step,
        Some(NextStep::ResolveConflicts)
    );
    assert_eq!(
        pull_request(&decisions, "acme/api", 1).held_back,
        lands_with(&["TEAM-1"])
    );
}

/// Acceptance: a held member with conflicts doesn't block Land, is listed as
/// "held; not blocking", and keeps Resolve conflicts on its own row.
#[test]
fn a_held_member_with_conflicts_does_not_block_land_and_keeps_resolve_conflicts() {
    let observation = acme_owner()
        .pull_request(ready("acme/api", 1))
        .pull_request(ready("acme/web", 3).conflicting())
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/web", 3),
        )
        .build();
    let decisions = decide_all(&observation, &acme());

    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, Some(NextStep::Land), "{issue:#?}");
    assert_says(
        issue,
        &[
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "held; not blocking: acme/web#3 (Held Repo acme/web; its own Next Step Resolve conflicts)",
            "held members are not landed: acme/web#3",
        ],
    );
    let held = pull_request(&decisions, "acme/web", 3);
    assert_eq!(held.next_step, Some(NextStep::ResolveConflicts));
    assert_eq!(held.held_back, None);

    // A held member that is ready is held for release, never "lands with".
    let ready_held = acme_owner()
        .pull_request(ready("acme/api", 1))
        .pull_request(ready("acme/web", 3))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/web", 3),
        )
        .build();
    let decisions = decide_all(&ready_held, &acme());
    assert_eq!(
        pull_request(&decisions, "acme/web", 3).held_back,
        Some(HeldBack::ForRelease)
    );
    assert_eq!(linear(&decisions, "TEAM-1").next_step, Some(NextStep::Land));
}

/// Acceptance: unknown mergeability never gives Land (the member's unknown is
/// carried to the issue), and neither does a queued member, which is named.
#[test]
fn unknown_mergeability_or_a_queued_member_never_gives_land_and_names_the_queued_member() {
    let group = |app: PrBuilder| {
        acme_owner()
            .pull_request(ready("acme/api", 1))
            .pull_request(app)
            .linear_issue(
                linear_issue("acme", "id-1", "TEAM-1")
                    .attached("acme/api", 1)
                    .attached("acme/app", 2),
            )
            .build()
    };

    let unknown = decide_all(&group(ready("acme/app", 2).mergeability_unknown()), &acme());
    let issue = linear(&unknown, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        &[
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "blocking: acme/app#2",
            "acme/app#2: mergeability not computed yet by GitHub",
        ],
    );

    let queued = decide_all(&group(ready("acme/app", 2).queued()), &acme());
    let issue = linear(&queued, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
        &["blocking: acme/app#2: queued: in the merge queue, not merged yet"],
    );

    // Spec 0002 rule 1: no member is in a merge queue, held ones included.
    let held_queued = acme_owner()
        .pull_request(ready("acme/api", 1))
        .pull_request(ready("acme/web", 3).queued())
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/web", 3),
        )
        .build();
    let issue = decide_all(&held_queued, &acme());
    let issue = linear(&issue, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
    assert_says(issue, &["blocking: acme/web#3: queued"]);
}

/// Acceptance: ambiguous or not-surveyed members block Land, and so do
/// attachments or member facts den didn't read completely.
#[test]
fn ambiguous_not_surveyed_or_unread_members_block_land() {
    // Ambiguous: acme/app#2 is also attached to TEAM-2.
    let ambiguous = acme_owner()
        .pull_request(ready("acme/api", 1))
        .pull_request(ready("acme/app", 2))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2").attached("acme/app", 2))
        .build();
    let decisions = decide_all(&ambiguous, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
        &["blocking: acme/app#2: ambiguous, also attached to TEAM-2"],
    );
    assert_eq!(
        pull_request(&decisions, "acme/app", 2).held_back,
        lands_with(&["TEAM-1", "TEAM-2"]),
        "an ambiguous member lands with both, and merges by itself with neither"
    );

    // Not surveyed: a member in a Repo den didn't survey.
    let unseen = acme_owner()
        .pull_request(ready("acme/api", 1))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/infra", 9),
        )
        .build();
    let issue = decide_all(&unseen, &acme());
    let issue = linear(&issue, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
        &["blocking: acme/infra#9: member not surveyed (den didn't survey its Repo)"],
    );

    // Attachments not all read: there may be a member den doesn't know.
    let partial_attachments = acme_owner()
        .pull_request(ready("acme/api", 1))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attachments_incomplete("more than 50 attachments"),
        )
        .build();
    let issue = decide_all(&partial_attachments, &acme());
    let issue = linear(&issue, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        &["den couldn't read all of the Linear Issue's attachments (more than 50 attachments)"],
    );

    // Member coverage: acme/app's PR details weren't all read.
    let partial_member = acme_owner()
        .pull_request(ready("acme/api", 1))
        .pull_request(ready("acme/app", 2))
        .repo_coverage(
            "acme/app",
            SourceKind::PullRequestDetails,
            SourceState::Partial(Incompleteness::FollowUpTruncated {
                what: "#7 (checks)".to_owned(),
            }),
        )
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_all(&partial_member, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        &[
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "blocking: acme/app#2: den didn't read all of its facts: Pull Request details of acme/app partial",
        ],
    );
}

/// Every member held: nothing would land, so no Land; and Mark for release
/// waits for held Pull Requests, so no Mark for release either.
#[test]
fn an_open_held_member_keeps_the_issue_from_mark_for_release() {
    let observation = acme_owner()
        .pull_request(ready("acme/web", 3))
        .member_state(
            "acme/api",
            1,
            PullRequestState::Merged {
                merged_at: Some(ts("2026-11-18T09:00:00Z")),
            },
        )
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/web", 3),
        )
        .build();
    let decisions = decide_all(&observation, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_eq!(
        issue.landing.members[0].status,
        MemberStatus::Merged {
            merged_at: Some(ts("2026-11-18T09:00:00Z"))
        }
    );
    assert_says(
        issue,
        &[
            "every open member is in a Held Repo, so nothing would land (and Mark for release waits for held Pull Requests)",
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "held; not blocking: acme/web#3 (Held Repo acme/web; held for release)",
        ],
    );
}

/// Acceptance: no open members, one read merged and one closed, state
/// neither the release state nor completed or canceled: Mark for release,
/// Waiting On the Linear assignee, with the members as evidence and the
/// deployments caveat.
#[test]
fn landed_members_give_mark_for_release_with_the_evidence() {
    let observation = acme_owner()
        .member_state("acme/api", 1, PullRequestState::Merged { merged_at: None })
        .member_state("acme/app", 2, PullRequestState::Closed)
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_all(&observation, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(
        issue.next_step,
        Some(NextStep::MarkForRelease),
        "{issue:#?}"
    );
    assert_eq!(issue.waiting_on, WaitingOn::Me);
    assert!(!issue.unattributed);
    assert_says(
        issue,
        &[
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "no open Pull Request is attached, and acme/api#1 merged",
            "it is in In Progress (started), neither the release state Ready for Release nor completed or canceled",
            "evidence: merged acme/api#1",
            "evidence: closed without merging acme/app#2",
            "den can't see deployments",
        ],
    );
    // Its coverage includes the member states it rests on.
    assert!(
        issue.coverage.iter().any(|c| c.source.is_member_states()),
        "{:?}",
        issue.coverage
    );
}

/// Acceptance: an issue already in the release state has no Next Step:
/// "release is outside den", Waiting On nobody, unattributed. That holds
/// with landed members and without any.
#[test]
fn an_issue_in_the_release_state_has_no_next_step_and_is_unattributed() {
    let landed = acme_owner()
        .member_state("acme/api", 1, PullRequestState::Merged { merged_at: None })
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .state("ready for release", LinearStateKind::Started)
                .attached("acme/api", 1),
        )
        .linear_issue(
            linear_issue("acme", "id-2", "TEAM-2").state(RELEASE, LinearStateKind::Started),
        )
        .build();
    let decisions = decide_all(&landed, &acme());
    for identifier in ["TEAM-1", "TEAM-2"] {
        let issue = linear(&decisions, identifier);
        assert_eq!(issue.next_step, None, "{issue:#?}");
        assert_eq!(issue.waiting_on, WaitingOn::Nobody);
        assert!(issue.unattributed, "{issue:?}");
        assert!(
            issue
                .explanation
                .no_step_reason
                .as_deref()
                .is_some_and(|r| r.ends_with("; release is outside den")),
            "{issue:?}"
        );
    }
}

/// Mark for release never rests on a guess: a member den hasn't read the
/// state of, one closed without merging, one GitHub reported open again, a
/// completed issue, and a workflow without a release state each leave the
/// issue without a Next Step, saying why.
#[test]
fn mark_for_release_needs_a_member_read_merged_and_a_release_state() {
    let attached = |builder: ObservationBuilder| {
        builder
            .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("acme/api", 1))
            .build()
    };

    let unread = decide_all(&attached(acme_owner()), &acme());
    let issue = linear(&unread, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_eq!(issue.landing.members[0].status, MemberStatus::NotOpen);
    assert_says(
        issue,
        &[
            "den withholds Mark for release until it has read that one of them merged",
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "whether acme/api#1 merged or was closed without merging: den hasn't read its state yet",
        ],
    );

    let failed = attached(acme_owner().owner_coverage(
        "acme",
        SourceKind::MemberStates,
        SourceState::Failed {
            error: "timeout".to_owned(),
        },
    ));
    let issue = decide_all(&failed, &acme());
    assert_says(
        linear(&issue, "TEAM-1"),
        &["Landing Group member states of acme failed: timeout"],
    );

    let closed = attached(acme_owner().member_state("acme/api", 1, PullRequestState::Closed));
    let issue = decide_all(&closed, &acme());
    let issue = linear(&issue, "TEAM-1");
    assert_eq!(issue.next_step, None);
    // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
    assert_says(issue, &["none merged: acme/api#1 closed without merging"]);

    let reopened = attached(acme_owner().member_state("acme/api", 1, PullRequestState::Open));
    let issue = decide_all(&reopened, &acme());
    let issue = linear(&issue, "TEAM-1");
    assert_eq!(issue.next_step, None);
    assert_says(issue, &["GitHub reported it open again"]);

    let merged =
        || acme_owner().member_state("acme/api", 1, PullRequestState::Merged { merged_at: None });
    let completed = merged()
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .state("Done", LinearStateKind::Completed)
                .attached("acme/api", 1),
        )
        .build();
    let issue = decide_all(&completed, &acme());
    let issue = linear(&issue, "TEAM-1");
    assert_eq!(issue.next_step, None);
    assert_says(issue, &["it is Done (completed): nothing is left for den"]);

    let no_release_state = Policies::default().with(
        "acme",
        LinearWorkflowPolicy {
            release_state: None,
            ..acme().for_owner("acme").clone()
        },
    );
    let issue = decide_all(&attached(merged()), &no_release_state);
    let issue = linear(&issue, "TEAM-1");
    assert_eq!(issue.next_step, None);
    assert_says(issue, &["this Linear Workflow names no release state"]);
}

/// A merged member doesn't block Land: only open members are landed.
#[test]
fn a_merged_member_does_not_block_land() {
    let observation = acme_owner()
        .pull_request(ready("acme/app", 2))
        .member_state("acme/api", 1, PullRequestState::Merged { merged_at: None })
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_all(&observation, &acme());
    assert_eq!(linear(&decisions, "TEAM-1").next_step, Some(NextStep::Land));
}

/// A Pull Request attached to another Owner's Linear Issue is no member of
/// that group (a Landing Group's Pull Requests are its Owner's): it keeps
/// Merge, and the issue reports it not surveyed.
#[test]
fn a_pr_attached_to_another_owners_linear_issue_keeps_merge() {
    let observation = acme_owner()
        .fully_read_owner("octo")
        .repo("octo/api")
        .pull_request(ready("octo/api", 4))
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("octo/api", 4))
        .build();
    let decisions = decide_all(&observation, &acme());
    let octo = pull_request(&decisions, "octo/api", 4);
    assert_eq!(octo.next_step, Some(NextStep::Merge), "{octo:#?}");
    assert_eq!(octo.held_back, None);
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None);
    assert_says(issue, &["member not surveyed"]);
}

/// A Merge PR of a workflow Owner whose Linear Issue lists weren't all read
/// might be a member den didn't see: it still shows Merge, but the Linear
/// source joins its coverage and the Explanation says what den couldn't
/// establish. With everything read, its Decision is unchanged.
#[test]
fn a_merge_pr_says_when_den_could_not_establish_its_landing_group() {
    let lists_partial = acme_owner()
        .owner_coverage(
            "acme",
            SourceKind::Linear(LinearSourceKind::AssignedIssues),
            SourceState::Failed {
                error: "timeout".to_owned(),
            },
        )
        .pull_request(ready("acme/api", 1))
        .build();
    let decisions = decide_all(&lists_partial, &acme());
    let merge = pull_request(&decisions, "acme/api", 1);
    assert_eq!(merge.next_step, Some(NextStep::Merge), "{merge:#?}");
    assert_says(
        merge,
        &[
            "whether it lands with a Linear Issue (a Landing Group member never merges by itself): assigned Linear Issues of acme failed: timeout",
        ],
    );
    assert!(
        merge.coverage.iter().any(|c| c.source
            == Source::Owner {
                owner: "acme".to_owned(),
                kind: SourceKind::Linear(LinearSourceKind::AssignedIssues),
            }),
        "{:?}",
        merge.coverage
    );

    let unread_attachments = acme_owner()
        .pull_request(ready("acme/api", 1))
        .linear_issue(
            linear_issue("acme", "id-3", "TEAM-3").attachments_incomplete("errors on the page"),
        )
        .build();
    let decisions = decide_all(&unread_attachments, &acme());
    assert_says(
        pull_request(&decisions, "acme/api", 1),
        &["TEAM-3's attachments weren't all read"],
    );

    let all_read = acme_owner().pull_request(ready("acme/api", 1)).build();
    let decisions = decide_all(&all_read, &acme());
    let merge = pull_request(&decisions, "acme/api", 1);
    assert_eq!(merge.next_step, Some(NextStep::Merge));
    assert!(merge.explanation.unknowns.is_empty(), "{merge:?}");
}

/// A member GitHub reported open again when den read its state after the
/// survey is open, though the survey didn't list it: den hasn't read its
/// facts. It keeps the issue from Mark for release (no open member may
/// remain) and blocks Land (it can't be judged ready), and is named.
#[test]
fn a_member_reported_open_again_blocks_mark_for_release_and_land() {
    let merged_and_reopened = acme_owner()
        .member_state("acme/api", 1, PullRequestState::Merged { merged_at: None })
        .member_state("acme/app", 2, PullRequestState::Open)
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_all(&merged_and_reopened, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_eq!(issue.landing.members[1].status, MemberStatus::OpenAgain);
    assert_says(
        issue,
        // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
        &["blocking: acme/app#2: GitHub reported it open again"],
    );

    let ready_and_reopened = acme_owner()
        .pull_request(ready("acme/api", 1))
        .member_state("acme/app", 2, PullRequestState::Open)
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/app", 2),
        )
        .build();
    let decisions = decide_all(&ready_and_reopened, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
        &["blocking: acme/app#2: GitHub reported it open again"],
    );

    // Held: reported open again in a Held Repo, it never blocks Land, but it
    // is open, so it still keeps the issue from Mark for release.
    let held_reopened = acme_owner()
        .pull_request(ready("acme/api", 1))
        .member_state("acme/web", 3, PullRequestState::Open)
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .attached("acme/web", 3),
        )
        .build();
    let decisions = decide_all(&held_reopened, &acme());
    assert_eq!(linear(&decisions, "TEAM-1").next_step, Some(NextStep::Land));
}

/// A held member never blocks Land, ambiguous or not: TEAM-1's Land would
/// never land it. It is listed as held, and its ambiguity is still said.
#[test]
fn an_ambiguous_held_member_does_not_block_land() {
    let observation = acme_owner()
        .pull_request(ready("acme/api", 4))
        .pull_request(ready("acme/web", 3))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/web", 3)
                .attached("acme/api", 4),
        )
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2").attached("acme/web", 3))
        .build();
    let decisions = decide_all(&observation, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, Some(NextStep::Land), "{}", said(issue));
    assert_says(
        issue,
        &[
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "held; not blocking: acme/web#3 (Held Repo acme/web; held for release; ambiguous, also attached to TEAM-2)",
        ],
    );
}

/// A member den hasn't asked about yet, after an earlier read that was
/// complete, reads "den hasn't read its state yet": GitHub was never asked,
/// so den doesn't say GitHub didn't report it.
#[test]
fn a_member_not_asked_yet_after_a_complete_read_is_not_blamed_on_github() {
    let observation = acme_owner()
        .owner_coverage("acme", SourceKind::MemberStates, SourceState::Complete)
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("acme/api", 7))
        .build();
    let decisions = decide_all(&observation, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_says(
        issue,
        &[
            // Merge of #49: members are spelled `owner/repo#n` (was `owner/repo #n`).
            "whether acme/api#7 merged or was closed without merging: den hasn't read its state yet",
        ],
    );
    assert!(
        !said(issue).contains("GitHub didn't report its state"),
        "{}",
        said(issue)
    );
}

/// With the Linear Issue lists read completely but one issue's attachments
/// not, a Merge PR might be a member den didn't see: the list that listed
/// that issue joins its coverage as partial, naming the issue, so its row
/// reads incomplete, not just its unknowns.
#[test]
fn a_merge_pr_is_incomplete_when_an_issues_attachments_were_not_all_read() {
    let observation = acme_owner()
        .owner_coverage(
            "acme",
            SourceKind::Linear(LinearSourceKind::AssignedIssues),
            SourceState::Complete,
        )
        .pull_request(ready("acme/api", 1))
        .linear_issue(
            linear_issue("acme", "id-3", "TEAM-3")
                .attachments_incomplete("more than 50 attachments"),
        )
        .build();
    let decisions = decide_all(&observation, &acme());
    let merge = pull_request(&decisions, "acme/api", 1);
    assert_eq!(merge.next_step, Some(NextStep::Merge), "{merge:#?}");
    let assigned = Source::Owner {
        owner: "acme".to_owned(),
        kind: SourceKind::Linear(LinearSourceKind::AssignedIssues),
    };
    let entry = merge
        .coverage
        .iter()
        .find(|c| c.source == assigned)
        .unwrap_or_else(|| panic!("{:?}", merge.coverage));
    assert_eq!(
        entry.state,
        SourceState::Partial(Incompleteness::FollowUpTruncated {
            what: "the attachments of TEAM-3 (more than 50 attachments)".to_owned(),
        })
    );
}

/// The Owner's member-state read bears only on a Linear Issue with a member
/// whose end den reads by number (merged or closed, not read yet, or read
/// closed). One member den can't read leaves that issue incomplete, never
/// the Owner's other Linear Issues: an Implement with no members, or a Mark
/// for release whose members are all read merged.
#[test]
fn a_failed_member_state_read_marks_only_the_issues_that_rest_on_it() {
    let partial = SourceState::Partial(Incompleteness::FollowUpLost {
        what: "the state of acme/api #99".to_owned(),
        reason: "GitHub returned no such Pull Request".to_owned(),
    });
    let observation = acme_owner()
        .owner_coverage("acme", SourceKind::MemberStates, partial)
        .member_state("acme/app", 2, PullRequestState::Merged { merged_at: None })
        .linear_issue(linear_issue("acme", "id-1", "TEAM-1").attached("acme/api", 99))
        .linear_issue(linear_issue("acme", "id-2", "TEAM-2").attached("acme/app", 2))
        .linear_issue(linear_issue("acme", "id-3", "TEAM-3"))
        .build();
    let decisions = decide_all(&observation, &acme());
    let member_states = |d: &Decision| d.coverage.iter().any(|c| c.source.is_member_states());

    let unread = linear(&decisions, "TEAM-1");
    assert!(member_states(unread), "{:?}", unread.coverage);

    let landed = linear(&decisions, "TEAM-2");
    assert_eq!(landed.next_step, Some(NextStep::MarkForRelease));
    assert!(!member_states(landed), "{:?}", landed.coverage);

    let implement = linear(&decisions, "TEAM-3");
    assert_eq!(implement.next_step, Some(NextStep::Implement));
    assert!(!member_states(implement), "{:?}", implement.coverage);
}

/// Where #49's fixes meet #37's rules (merge of integrate/m2-wave3). A link
/// that looks like a Pull Request den can't read withholds Mark for release,
/// as an unread attachment does: den can't tell that no member is open. And
/// a member reported open again is open, so the link's unknown says only
/// that more Pull Requests may be attached (`MemberStatus::is_open`).
#[test]
fn a_pull_request_like_link_withholds_mark_for_release_and_a_member_open_again_is_open() {
    const LINK: &str = "github.com/acme/api/pull/8";
    let merged_and_link = acme_owner()
        .member_state("acme/api", 1, PullRequestState::Merged { merged_at: None })
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .link(LINK),
        )
        .build();
    let decisions = decide_all(&merged_and_link, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        &[&format!(
            "den couldn't establish whether a PR exists: attached {LINK} looks like a Pull Request it can't read"
        )],
    );

    let reopened_and_link = acme_owner()
        .member_state("acme/app", 2, PullRequestState::Open)
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/app", 2)
                .link(LINK),
        )
        .build();
    let decisions = decide_all(&reopened_and_link, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.landing.members[0].status, MemberStatus::OpenAgain);
    assert_says(
        issue,
        &[&format!(
            "more Pull Requests may be attached: attached {LINK} looks like a Pull Request den can't read"
        )],
    );
    assert!(
        !said(issue).contains("couldn't establish whether a PR exists"),
        "{}",
        said(issue)
    );
}

/// A link that looks like a Pull Request den can't read blocks Land, as an
/// unread attachment list does: it may be a member den can't judge, so a
/// possibly incomplete group is never presented as ready. The ready member
/// still lands with the issue.
#[test]
fn a_pull_request_like_link_blocks_land() {
    const LINK: &str = "github.com/acme/api/pull/8";
    let observation = acme_owner()
        .pull_request(ready("acme/api", 1))
        .linear_issue(
            linear_issue("acme", "id-1", "TEAM-1")
                .attached("acme/api", 1)
                .link(LINK),
        )
        .build();
    let decisions = decide_all(&observation, &acme());
    let issue = linear(&decisions, "TEAM-1");
    assert_eq!(issue.next_step, None, "{issue:#?}");
    assert_says(
        issue,
        &[
            "1 blocking",
            &format!(
                "blocking: attached {LINK} looks like a Pull Request den can't read, so it can't tell every member"
            ),
        ],
    );
    assert_eq!(
        pull_request(&decisions, "acme/api", 1).held_back,
        lands_with(&["TEAM-1"])
    );
}

/// Whether a Linear Issue gets a group decision never depends on Start:
/// an unassigned issue the workflow no longer offers (here it was started,
/// as after a config or state change, with its facts kept) still decides
/// for its group. A ready member's Merge is suppressed ("lands with"), so the
/// issue must show Land; and one whose members merged shows Mark for
/// release. Without members it stays hidden, as before.
#[test]
fn an_issue_not_offered_to_start_still_decides_for_its_group() {
    let not_offered = |id: &str, identifier: &str| {
        linear_issue("acme", id, identifier)
            .unassigned()
            .state("In Progress", LinearStateKind::Started)
    };
    let observation = acme_owner()
        .pull_request(ready("acme/api", 1))
        .member_state("acme/app", 2, PullRequestState::Merged { merged_at: None })
        .linear_issue(not_offered("id-1", "TEAM-1").attached("acme/api", 1))
        .linear_issue(not_offered("id-2", "TEAM-2").attached("acme/app", 2))
        .linear_issue(not_offered("id-3", "TEAM-3"))
        .build();
    let decisions = decide_all(&observation, &acme());

    assert_eq!(
        pull_request(&decisions, "acme/api", 1).held_back,
        lands_with(&["TEAM-1"])
    );
    let land = linear(&decisions, "TEAM-1");
    assert_eq!(land.next_step, Some(NextStep::Land), "{land:#?}");
    assert_says(land, &["not offered to Start"]);

    let release = linear(&decisions, "TEAM-2");
    assert_eq!(
        release.next_step,
        Some(NextStep::MarkForRelease),
        "{release:#?}"
    );

    assert!(
        !decisions
            .iter()
            .any(|d| d.item.identifier.as_deref() == Some("TEAM-3")),
        "an unoffered issue without members stays hidden"
    );
}
