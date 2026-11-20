//! Correlation (spec 0002, "Correlation"; #35): the Landing Groups of an
//! Observation, built before any rule runs and before Scope filtering, so a
//! group never depends on what the list shows. Pure.
//!
//! - **Attached**: a Linear attachment whose URL is a Pull Request. It is the
//!   only thing that makes a member. A Pull Request attached to two Linear
//!   Issues is a member of both, and **ambiguous** in both.
//! - **Hint**: an open Pull Request of the Linear Issue's Owner whose branch
//!   name contains the identifier on word boundaries (`TEAM-1` matches
//!   `me/team-1-fix`, never `team-12-fix`). Shown as "possibly for"; never a
//!   member, so it can't hold up, authorize or replace anything.
//! - A member is **not surveyed** when den can't see it: a Pull Request in
//!   another Owner's Repo (a Landing Group's Pull Requests are its Owner's),
//!   in a Repo den didn't survey, or in one whose open Pull Requests den
//!   didn't read completely. A member den looked for in a completely read
//!   Repo and didn't find isn't open: merged or closed. The survey reads open
//!   Pull Requests only, so den says which only once it has read that
//!   member's state by number (`Observation.member_states`, #37): **merged**,
//!   **closed** without merging, or still "merged or closed" until then.
//!
//! Pull Requests are identified by Owner, Repo and number, compared as GitHub
//! does (case-insensitively), so two Owners' `api#42` never meet; Linear
//! Issues by workspace and permanent ID, so an issue moved to another team
//! keeps its group.

use crate::domain::{
    CoverageEntry, LinearIssueId, Observation, RepoId, Source, SourceKind, WorkItemId,
    facts::{LinearIssue, PullRequest, PullRequestRef, PullRequestState},
    ids::same_login,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Where one Work Item stands in Landing Groups, carried on its Decision. A
/// Linear Issue fills `members` and `hints`; a Pull Request fills `groups`
/// and `possibly_for`. Every other Work Item, and every one of an
/// Observation without Linear Issues, has the empty one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Landing {
    /// A Linear Issue's members: the Pull Requests attached to it, in
    /// Linear's order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<Member>,
    /// A Linear Issue's hints: its Owner's open Pull Requests whose branch
    /// name contains its identifier, not attached to it. Never members.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<Hint>,
    /// A Pull Request's Landing Groups: the Linear Issues it is attached to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupRef>,
    /// A Pull Request's hints: the Linear Issues of its Owner its branch
    /// name names, that it isn't attached to ("possibly for TEAM-123").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub possibly_for: Vec<GroupRef>,
}

impl Landing {
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
            && self.hints.is_empty()
            && self.groups.is_empty()
            && self.possibly_for.is_empty()
    }

    /// A Pull Request attached to more than one Linear Issue.
    pub fn ambiguous(&self) -> bool {
        self.groups.len() > 1
    }
}

/// A Linear Issue, named for a Pull Request's row and detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRef {
    /// The Linear Issue's Work Item identity (workspace + permanent ID).
    pub issue: WorkItemId,
    /// `TEAM-123`, for display.
    pub identifier: String,
}

/// One member of a Landing Group: an attached Pull Request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub pr: PullRequestRef,
    pub status: MemberStatus,
    /// The other Linear Issues it is attached to; any makes it ambiguous.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_attached_to: Vec<GroupRef>,
}

impl Member {
    pub fn ambiguous(&self) -> bool {
        !self.also_attached_to.is_empty()
    }

    /// `acme/api #4`.
    pub fn reference(&self) -> String {
        pull_request_reference(&self.pr)
    }
}

/// What den knows of a member Pull Request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemberStatus {
    /// Open, and in the survey: its own Decision says what it needs.
    Open { draft: bool },
    /// Its Repo's open Pull Requests were read completely and it isn't among
    /// them: merged or closed, and den hasn't read which (or GitHub reported
    /// it open again when den asked).
    NotOpen,
    /// Not open, and GitHub reported it merged when den read its state (#37).
    Merged { merged_at: Option<Timestamp> },
    /// Not open, and GitHub reported it closed without merging (#37).
    Closed,
    /// den can't see it, for this reason.
    NotSurveyed(NotSurveyed),
}

impl MemberStatus {
    /// "open", "draft", "merged or closed", "merged", "closed without
    /// merging", "member not surveyed".
    pub fn label(&self) -> &'static str {
        match self {
            Self::Open { draft: false } => "open",
            Self::Open { draft: true } => "draft",
            Self::NotOpen => "merged or closed",
            Self::Merged { .. } => "merged",
            Self::Closed => "closed without merging",
            Self::NotSurveyed(_) => "member not surveyed",
        }
    }

    /// Not open, whether or not den has read how it ended.
    pub fn is_not_open(&self) -> bool {
        matches!(self, Self::NotOpen | Self::Merged { .. } | Self::Closed)
    }

    /// The label and, when den can't see the member, why.
    pub fn describe(&self) -> String {
        match self {
            Self::NotSurveyed(why) => format!("{}: {}", self.label(), why.describe()),
            _ => self.label().to_owned(),
        }
    }
}

/// Why den can't see a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotSurveyed {
    /// In another Owner's Repo: a Landing Group's Pull Requests are its
    /// Owner's.
    OtherOwner,
    /// In a Repo that isn't among the Owner's Repos den read.
    RepoNotSurveyed,
    /// den didn't read the Repo's open Pull Requests completely.
    PullRequestsIncomplete,
}

impl NotSurveyed {
    pub fn describe(self) -> &'static str {
        match self {
            Self::OtherOwner => {
                "it is in another Owner's Repo, and a Landing Group's Pull Requests are its Owner's"
            }
            Self::RepoNotSurveyed => "den didn't survey its Repo",
            Self::PullRequestsIncomplete => {
                "den didn't read its Repo's open Pull Requests completely"
            }
        }
    }
}

/// A Pull Request whose branch name looks like a Linear Issue's identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hint {
    pub pr: PullRequestRef,
    /// Its head branch.
    pub branch: String,
}

impl Hint {
    /// `acme/api #4`.
    pub fn reference(&self) -> String {
        pull_request_reference(&self.pr)
    }
}

/// `acme/api #4`.
pub fn pull_request_reference(pr: &PullRequestRef) -> String {
    format!("{} #{}", pr.repo, pr.number)
}

/// The Landing Groups of one Observation, looked up per Work Item.
#[derive(Debug, Clone, Default)]
pub struct Correlation {
    issues: Vec<(LinearIssueId, Landing)>,
    pull_requests: Vec<(PullRequestRef, Landing)>,
}

impl Correlation {
    /// A Linear Issue's group: its members and hints.
    pub fn for_issue(&self, issue: &LinearIssueId) -> Landing {
        self.issues
            .iter()
            .find(|(id, _)| id == issue)
            .map(|(_, landing)| landing.clone())
            .unwrap_or_default()
    }

    /// A Pull Request's groups and hints.
    pub fn for_pull_request(&self, repo: &RepoId, number: u64) -> Landing {
        self.pull_requests
            .iter()
            .find(|(pr, _)| pr.is(repo, number))
            .map(|(_, landing)| landing.clone())
            .unwrap_or_default()
    }
}

/// Builds every Landing Group of `observation` (see the module docs).
pub fn correlate(observation: &Observation) -> Correlation {
    let issues = distinct_issues(&observation.linear.issues);
    let attached: Vec<(&LinearIssue, Vec<PullRequestRef>)> = issues
        .iter()
        .map(|issue| (*issue, attached_pull_requests(issue)))
        .collect();
    let attachers = |pr: &PullRequestRef| -> Vec<GroupRef> {
        attached
            .iter()
            .filter(|(_, prs)| prs.iter().any(|p| p.is(&pr.repo, pr.number)))
            .map(|(issue, _)| group_ref(issue))
            .collect()
    };

    let issue_groups = attached
        .iter()
        .map(|(issue, prs)| {
            let this = issue.work_item_id();
            let members = prs
                .iter()
                .map(|pr| Member {
                    pr: pr.clone(),
                    status: status(observation, &issue.owner, pr),
                    also_attached_to: attachers(pr)
                        .into_iter()
                        .filter(|g| g.issue != this)
                        .collect(),
                })
                .collect();
            let hints = observation
                .pull_requests
                .iter()
                .filter(|pr| same_login(&pr.repo.owner, &issue.owner))
                .filter(|pr| !prs.iter().any(|p| p.is(&pr.repo, pr.number)))
                .filter(|pr| names_identifier(&pr.head_ref, &issue.identifier))
                .map(|pr| Hint {
                    pr: reference_of(pr),
                    branch: pr.head_ref.clone(),
                })
                .collect();
            (
                issue.id.clone(),
                Landing {
                    members,
                    hints,
                    ..Landing::default()
                },
            )
        })
        .collect();

    let pull_request_groups = observation
        .pull_requests
        .iter()
        .map(|pr| {
            let this = reference_of(pr);
            let groups = attachers(&this);
            let possibly_for = attached
                .iter()
                .filter(|(issue, prs)| {
                    same_login(&pr.repo.owner, &issue.owner)
                        && !prs.iter().any(|p| p.is(&pr.repo, pr.number))
                        && names_identifier(&pr.head_ref, &issue.identifier)
                })
                .map(|(issue, _)| group_ref(issue))
                .collect();
            (
                this,
                Landing {
                    groups,
                    possibly_for,
                    ..Landing::default()
                },
            )
        })
        .filter(|(_, landing)| !landing.is_empty())
        .collect();

    Correlation {
        issues: issue_groups,
        pull_requests: pull_request_groups,
    }
}

/// Each Linear Issue once, by identity (the first one read wins).
fn distinct_issues(issues: &[LinearIssue]) -> Vec<&LinearIssue> {
    let mut distinct: Vec<&LinearIssue> = Vec::new();
    for issue in issues {
        if !distinct.iter().any(|seen| seen.id == issue.id) {
            distinct.push(issue);
        }
    }
    distinct
}

/// The Pull Requests attached to `issue`, each once.
fn attached_pull_requests(issue: &LinearIssue) -> Vec<PullRequestRef> {
    let mut prs: Vec<PullRequestRef> = Vec::new();
    for pr in issue.attachments.pull_requests() {
        if !prs.iter().any(|p| p.is(&pr.repo, pr.number)) {
            prs.push(pr.clone());
        }
    }
    prs
}

fn group_ref(issue: &LinearIssue) -> GroupRef {
    GroupRef {
        issue: issue.work_item_id(),
        identifier: issue.identifier.clone(),
    }
}

fn reference_of(pr: &PullRequest) -> PullRequestRef {
    PullRequestRef::new(pr.repo.clone(), pr.number)
}

/// What den knows of `pr`, a member of a Linear Issue of `owner`.
fn status(observation: &Observation, owner: &str, pr: &PullRequestRef) -> MemberStatus {
    if !same_login(&pr.repo.owner, owner) {
        return MemberStatus::NotSurveyed(NotSurveyed::OtherOwner);
    }
    if let Some(found) = observation
        .pull_requests
        .iter()
        .find(|p| pr.is(&p.repo, p.number))
    {
        return MemberStatus::Open {
            draft: found.is_draft,
        };
    }
    if !observation.repos.iter().any(|r| r.id.same(&pr.repo)) {
        return MemberStatus::NotSurveyed(NotSurveyed::RepoNotSurveyed);
    }
    if !open_pull_requests_read_completely(observation.coverage.entries(), &pr.repo) {
        return MemberStatus::NotSurveyed(NotSurveyed::PullRequestsIncomplete);
    }
    let read = observation
        .member_states
        .iter()
        .find(|m| m.pr.is(&pr.repo, pr.number));
    match read.map(|m| &m.state) {
        Some(PullRequestState::Merged { merged_at }) => MemberStatus::Merged {
            merged_at: *merged_at,
        },
        Some(PullRequestState::Closed) => MemberStatus::Closed,
        // Open again, or not read yet: den can't say how it ended.
        Some(PullRequestState::Open) | None => MemberStatus::NotOpen,
    }
}

/// Whether den read every open Pull Request of `repo`: its Owner's Pull
/// Request list was recorded complete, and so was the Repo's own, if it had
/// one.
fn open_pull_requests_read_completely(entries: &[CoverageEntry], repo: &RepoId) -> bool {
    let mut owner_complete = false;
    for entry in entries {
        match &entry.source {
            Source::Owner {
                owner,
                kind: SourceKind::PullRequests,
            } if same_login(owner, &repo.owner) => {
                if !entry.state.is_complete() {
                    return false;
                }
                owner_complete = true;
            }
            Source::Repo {
                repo: r,
                kind: SourceKind::PullRequests,
            } if r.same(repo) && !entry.state.is_complete() => return false,
            _ => {}
        }
    }
    owner_complete
}

/// Whether `branch` contains `identifier` on word boundaries, ignoring case:
/// the characters on either side, if any, are neither letters nor digits.
/// `TEAM-1` is in `me/team-1-fix` and `TEAM-1`, never in `team-12-fix` or
/// `steam-1`.
fn names_identifier(branch: &str, identifier: &str) -> bool {
    if identifier.is_empty() {
        return false;
    }
    // ASCII lowercasing keeps every byte offset.
    let branch = branch.to_ascii_lowercase();
    let identifier = identifier.to_ascii_lowercase();
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    branch.match_indices(&identifier).any(|(start, found)| {
        let before = branch.get(..start).and_then(|s| s.chars().next_back());
        let after = branch
            .get(start + found.len()..)
            .and_then(|s| s.chars().next());
        boundary(before) && boundary(after)
    })
}
