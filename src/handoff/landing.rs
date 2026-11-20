//! A Land Job's Landing Group (#40, spec 0002 "Landing a group"), pure: from
//! a Linear Issue's Land Decision, the Observation and the Owner's Linear
//! Workflow policy, the members to land in landing order, each pinned to the
//! head commit den saw, and the held members it must leave alone.
//!
//! - **Who lands:** the open members outside Held Repos. A member that is no
//!   longer open (merged or closed) has nothing left to land; a held member
//!   is listed as "not landed: held" and never given to the Job to merge.
//! - **In what order:** members in the workflow's `landing_order` Repos
//!   first, in that order; every other member after them, by Repo name and
//!   then number, so the order never depends on Linear's attachment order.
//! - **The worktree:** on the first member's PR head, as for a PR Job, but on
//!   a new local branch named for the Linear Issue (`land_team-2`), so it
//!   never clashes with a member's own coding Job on its head branch.
//!
//! The plan carries this list; `Enter` re-plans and asks again when it
//! differs ([`LandingGroupPlan::change_from`]), and the executor re-reads every
//! member's head from GitHub before anything is done.

use super::{BaseRef, Refusal, RefusalReason, TargetChoice, kinds, names};
use crate::{
    decide::Decision,
    domain::{
        LinearWorkflowPolicy, NextStep, Observation, RepoId, WorkItemId,
        facts::{CheckOutcome, PrDetails, PullRequestRef},
    },
    jobs::{LandedMember, LandingGroupSubject},
};
use serde::{Deserialize, Serialize};

/// The Landing Group a Land Job is told to land, fixed when it is planned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandingGroupPlan {
    /// The Linear Issue (`WorkItemId::LinearIssue`).
    pub issue: WorkItemId,
    /// `TEAM-123`, for display.
    pub identifier: String,
    /// The members to land, in landing order. Never empty.
    pub members: Vec<LandingMember>,
    /// Open members in Held Repos: listed, never landed.
    pub held: Vec<HeldMember>,
}

/// One member to land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandingMember {
    pub pr: PullRequestRef,
    /// The Pull Request's URL on GitHub.
    pub url: String,
    /// Its Repo's URL on GitHub: where the executor re-reads its head.
    pub repo_url: String,
    /// The full head commit den saw: the one the Job merges, pinned.
    pub head_commit: String,
    /// Its checks, in words ("passing (2)", "not read").
    pub checks: String,
    /// Its place in the configured landing order; `None`: unlisted.
    pub configured: Option<usize>,
}

/// An open member in a Held Repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldMember {
    pub pr: PullRequestRef,
    pub url: String,
}

impl LandingGroupPlan {
    /// What a Land Job's record covers: the Linear Issue and every member it
    /// lands, at its approved head.
    pub fn subject(&self) -> LandingGroupSubject {
        LandingGroupSubject {
            issue: self.issue.clone(),
            identifier: self.identifier.clone(),
            members: self
                .members
                .iter()
                .map(|member| LandedMember {
                    pr: member.pr.clone(),
                    head: member.head_commit.clone(),
                })
                .collect(),
        }
    }

    /// What changed from `before` (the plan the sheet showed) to this one, in
    /// what the user approved: a member that joined or left, a head that
    /// moved, the order, the held members. `None` when nothing did.
    pub fn change_from(&self, before: &Self) -> Option<String> {
        let mut changes = Vec::new();
        for member in &self.members {
            match before.members.iter().find(|b| b.pr == member.pr) {
                None => changes.push(format!("{} joined", reference(&member.pr))),
                Some(was) if was.head_commit != member.head_commit => changes.push(format!(
                    "{}'s head moved from {} to {}",
                    reference(&member.pr),
                    super::short_commit(&was.head_commit),
                    super::short_commit(&member.head_commit)
                )),
                Some(_) => {}
            }
        }
        for was in &before.members {
            if !self.members.iter().any(|m| m.pr == was.pr) {
                changes.push(format!("{} left", reference(&was.pr)));
            }
        }
        let order = |plan: &Self| {
            plan.members
                .iter()
                .map(|m| m.pr.clone())
                .collect::<Vec<_>>()
        };
        if changes.is_empty() && order(self) != order(before) {
            changes.push("the landing order changed".to_owned());
        }
        let held = |plan: &Self| plan.held.iter().map(|m| m.pr.clone()).collect::<Vec<_>>();
        if held(self) != held(before) {
            changes.push("the held members changed".to_owned());
        }
        (!changes.is_empty()).then(|| {
            format!(
                "the Landing Group of {} changed ({})",
                self.identifier,
                changes.join("; ")
            )
        })
    }
}

/// `acme/api #1`.
pub fn reference(pr: &PullRequestRef) -> String {
    format!("{} #{}", pr.repo, pr.number)
}

/// The launch facts of a Land Decision: the Landing Group, and a worktree on
/// its first member's head; or every reason it can't be launched.
pub(super) fn facts(
    decision: &Decision,
    observation: &Observation,
    policy: &LinearWorkflowPolicy,
) -> Result<kinds::Facts, Refusal> {
    let group = group(decision, observation, policy)?;
    let mut reasons = Vec::new();
    let Some(first) = group.members.first() else {
        return Err(Refusal::one(RefusalReason::NothingToLand {
            identifier: group.identifier,
        }));
    };
    let Some(pr) = observation
        .pull_requests
        .iter()
        .find(|p| first.pr.is(&p.repo, p.number))
    else {
        return Err(Refusal::one(RefusalReason::LandingMemberGone {
            pr: first.pr.clone(),
        }));
    };
    let Some(repo_facts) = observation.repos.iter().find(|r| r.id.same(&pr.repo)) else {
        return Err(Refusal::one(RefusalReason::RepoUnknown {
            repo: pr.repo.clone(),
        }));
    };
    // A member's refs live in the Repo it is in: never a fork's Upstream.
    let target = kinds::chosen_target(repo_facts, TargetChoice::OwnRepo).map_err(Refusal::one)?;
    if target.archived {
        reasons.push(RefusalReason::RepoArchived {
            repo: target.target.repo.clone(),
        });
    }
    if !reasons.is_empty() {
        return Err(Refusal { reasons });
    }
    let base = if pr
        .head_repo
        .as_ref()
        .is_some_and(|head| head.same(&pr.repo))
    {
        BaseRef::HeadBranch(pr.head_ref.clone())
    } else {
        BaseRef::PullHead { number: pr.number }
    };
    Ok(kinds::Facts {
        step: NextStep::Land,
        target: target.target,
        number: Some(pr.number),
        base,
        base_commit: Some(first.head_commit.clone()),
        branch: names::new_branch_name(NextStep::Land, &names::slug(&group.identifier)),
        new_branch: true,
        landing: Some(group),
    })
}

/// The Landing Group of a Land Decision, or every reason den can't plan it.
fn group(
    decision: &Decision,
    observation: &Observation,
    policy: &LinearWorkflowPolicy,
) -> Result<LandingGroupPlan, Refusal> {
    let identifier = decision
        .item
        .identifier
        .clone()
        .unwrap_or_else(|| decision.item.id.to_string());
    let mut reasons = Vec::new();
    let mut members = Vec::new();
    let mut held = Vec::new();
    for member in decision.landing.members.iter().filter(|m| {
        matches!(
            m.status,
            crate::decide::correlation::MemberStatus::Open { .. }
        )
    }) {
        let Some(pr) = observation
            .pull_requests
            .iter()
            .find(|p| member.pr.is(&p.repo, p.number))
        else {
            reasons.push(RefusalReason::LandingMemberGone {
                pr: member.pr.clone(),
            });
            continue;
        };
        if policy.holds(&pr.repo) {
            held.push(HeldMember {
                pr: member.pr.clone(),
                url: pr.url.clone(),
            });
            continue;
        }
        let Some(repo) = observation.repos.iter().find(|r| r.id.same(&pr.repo)) else {
            reasons.push(RefusalReason::RepoUnknown {
                repo: pr.repo.clone(),
            });
            continue;
        };
        members.push(LandingMember {
            pr: member.pr.clone(),
            url: pr.url.clone(),
            repo_url: repo.url.clone(),
            head_commit: pr.head_commit.clone(),
            checks: checks_words(pr.details.as_ref()),
            configured: policy.landing_rank(&pr.repo),
        });
    }
    if !reasons.is_empty() {
        return Err(Refusal { reasons });
    }
    members.sort_by_key(|m| order_key(&m.pr.repo, m.pr.number, m.configured));
    held.sort_by_key(|m| order_key(&m.pr.repo, m.pr.number, None));
    if members.is_empty() {
        return Err(Refusal::one(RefusalReason::NothingToLand { identifier }));
    }
    Ok(LandingGroupPlan {
        issue: decision.item.id.clone(),
        identifier,
        members,
        held,
    })
}

/// Listed Repos first, in their configured order; then by folded Repo and
/// number.
fn order_key(repo: &RepoId, number: u64, configured: Option<usize>) -> (usize, RepoId, u64) {
    (configured.unwrap_or(usize::MAX), repo.folded(), number)
}

/// A member's checks, in words, as den read them.
fn checks_words(details: Option<&PrDetails>) -> String {
    let Some(details) = details else {
        return "checks not read".to_owned();
    };
    let checks = &details.checks.items;
    let count =
        |outcome: fn(&CheckOutcome) -> bool| checks.iter().filter(|c| outcome(&c.outcome)).count();
    let failing = count(|o| matches!(o, CheckOutcome::Failing));
    let pending = count(|o| matches!(o, CheckOutcome::Pending));
    let other = count(|o| matches!(o, CheckOutcome::Unrecognised(_)));
    let mut words = match (checks.len(), failing, pending, other) {
        (0, ..) => "no checks".to_owned(),
        (all, 0, 0, 0) => format!("checks passing ({all})"),
        (all, ..) => {
            let parts: Vec<String> = [
                (failing, "failing"),
                (pending, "pending"),
                (other, "in a state den doesn't know"),
            ]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, what)| format!("{n} {what}"))
            .collect();
            format!("checks: {} of {all}", parts.join(", "))
        }
    };
    if !details.checks.complete {
        words.push_str("; not all read");
    }
    words
}
