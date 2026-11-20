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
//!   **closed** without merging, or still "merged or closed" until then. One
//!   GitHub reports open at that read is **open again**: open, but not in
//!   the survey, so den hasn't read its facts.
//!
//! Pull Requests are identified by Owner, Repo and number, compared as GitHub
//! does (case-insensitively), so two Owners' `api#42` never meet; Linear
//! Issues by workspace and permanent ID, so an issue moved to another team
//! keeps its group.

use crate::domain::{
    LinearIssueId, Observation, RepoId, SourceKind, WorkItemId,
    facts::{LinearIssue, PullRequest, PullRequestRef, PullRequestState},
    ids::same_login,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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

    /// "ambiguous, also attached to TEAM-2, TEAM-3" when it is ambiguous: the
    /// one wording for the Explanation and the detail.
    pub fn ambiguity(&self) -> Option<String> {
        if !self.ambiguous() {
            return None;
        }
        let others: Vec<&str> = self
            .also_attached_to
            .iter()
            .map(|g| g.identifier.as_str())
            .collect();
        Some(format!("ambiguous, also attached to {}", others.join(", ")))
    }

    /// `acme/api#4`.
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
    /// them: merged or closed, and den hasn't read which.
    NotOpen,
    /// Missing from its Repo's open Pull Requests, but GitHub reported it
    /// open when den read its state after the survey (#37): open, and den
    /// hasn't read its facts. Until a survey lists it, it can't be judged.
    OpenAgain,
    /// Not open, and GitHub reported it merged when den read its state (#37).
    Merged { merged_at: Option<Timestamp> },
    /// Not open, and GitHub reported it closed without merging (#37).
    Closed,
    /// den can't see it, for this reason.
    NotSurveyed(NotSurveyed),
}

impl MemberStatus {
    /// "open", "draft", "merged or closed", "open again", "merged", "closed
    /// without merging", "member not surveyed".
    pub fn label(&self) -> &'static str {
        match self {
            Self::Open { draft: false } => "open",
            Self::Open { draft: true } => "draft",
            Self::NotOpen => "merged or closed",
            Self::OpenAgain => "open again",
            Self::Merged { .. } => "merged",
            Self::Closed => "closed without merging",
            Self::NotSurveyed(_) => "member not surveyed",
        }
    }

    /// Not open, whether or not den has read how it ended.
    pub fn is_not_open(&self) -> bool {
        matches!(self, Self::NotOpen | Self::Merged { .. } | Self::Closed)
    }

    /// Open: in the survey, or reported open again after it.
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. } | Self::OpenAgain)
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
    /// `acme/api#4`.
    pub fn reference(&self) -> String {
        pull_request_reference(&self.pr)
    }
}

/// `acme/api#4`: a Pull Request's Work Item identity as den shows it
/// everywhere (`WorkItemId`'s `Display`), so a member and the Pull Request's
/// own detail read alike.
pub fn pull_request_reference(pr: &PullRequestRef) -> String {
    pr.id().to_string()
}

/// The Landing Groups of one Observation, looked up per Work Item by
/// identity: a Linear Issue by workspace and permanent ID, a Pull Request by
/// its folded Owner, Repo and number.
#[derive(Debug, Clone, Default)]
pub struct Correlation {
    issues: HashMap<LinearIssueId, Landing>,
    pull_requests: HashMap<PullRequestKey, Landing>,
}

impl Correlation {
    /// A Linear Issue's group: its members and hints.
    pub fn for_issue(&self, issue: &LinearIssueId) -> Landing {
        self.issues.get(issue).cloned().unwrap_or_default()
    }

    /// A Pull Request's groups and hints.
    pub fn for_pull_request(&self, repo: &RepoId, number: u64) -> Landing {
        self.pull_requests
            .get(&PullRequestKey::of(repo, number))
            .cloned()
            .unwrap_or_default()
    }
}

/// A Pull Request's identity as GitHub compares it: its folded Work Item
/// identity ([`WorkItemId::folded`], the one case-insensitive rule). A map
/// key, so a lookup by Pull Request needn't scan.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PullRequestKey(WorkItemId);

impl PullRequestKey {
    pub fn of(repo: &RepoId, number: u64) -> Self {
        Self(
            WorkItemId::PullRequest {
                repo: repo.clone(),
                number,
            }
            .folded(),
        )
    }
}

/// What `status` and the hints look up, built once per `correlate`: the
/// open Pull Requests by identity, the surveyed Repos (folded), and each
/// open Pull Request's head branch lowercased.
struct Lookup<'o> {
    open: HashMap<PullRequestKey, &'o PullRequest>,
    repos: HashSet<RepoId>,
    branches: Vec<String>,
}

impl<'o> Lookup<'o> {
    fn of(observation: &'o Observation) -> Self {
        let mut open = HashMap::new();
        for pr in &observation.pull_requests {
            // The first one read wins, as everywhere in correlation.
            open.entry(PullRequestKey::of(&pr.repo, pr.number))
                .or_insert(pr);
        }
        Self {
            open,
            repos: observation.repos.iter().map(|r| r.id.folded()).collect(),
            branches: observation
                .pull_requests
                .iter()
                .map(|pr| pr.head_ref.to_ascii_lowercase())
                .collect(),
        }
    }
}

/// Builds every Landing Group of `observation` (see the module docs).
pub fn correlate(observation: &Observation) -> Correlation {
    let lookup = Lookup::of(observation);
    let issues = distinct_issues(&observation.linear.issues);
    let attached: Vec<(&LinearIssue, Vec<PullRequestRef>)> = issues
        .iter()
        .map(|issue| (*issue, attached_pull_requests(issue)))
        .collect();

    // Which Linear Issues each Pull Request is attached to. Only its own
    // Owner's: a Landing Group's Pull Requests are its Owner's, so another
    // Owner's attachment (a member "not surveyed" on the issue's side) puts
    // the Pull Request in no group and makes no member ambiguous.
    let mut attachers: HashMap<PullRequestKey, Vec<GroupRef>> = HashMap::new();
    for (issue, prs) in &attached {
        for pr in prs
            .iter()
            .filter(|pr| same_login(&pr.repo.owner, &issue.owner))
        {
            attachers
                .entry(PullRequestKey::of(&pr.repo, pr.number))
                .or_default()
                .push(group_ref(issue));
        }
    }

    // Every hint, found once and given to both sides: the issue's `hints`
    // and the Pull Request's `possibly_for`.
    let mut issue_hints: Vec<Vec<Hint>> = vec![Vec::new(); attached.len()];
    let mut pull_request_hints: Vec<Vec<GroupRef>> =
        vec![Vec::new(); observation.pull_requests.len()];
    for (i, (issue, prs)) in attached.iter().enumerate() {
        let identifier = issue.identifier.to_ascii_lowercase();
        for (p, (pr, branch)) in observation
            .pull_requests
            .iter()
            .zip(&lookup.branches)
            .enumerate()
        {
            if !is_hint(issue, prs, pr, branch, &identifier) {
                continue;
            }
            if let Some(hints) = issue_hints.get_mut(i) {
                hints.push(Hint {
                    pr: reference_of(pr),
                    branch: pr.head_ref.clone(),
                });
            }
            if let Some(possibly_for) = pull_request_hints.get_mut(p) {
                possibly_for.push(group_ref(issue));
            }
        }
    }

    let issue_groups = attached
        .iter()
        .zip(issue_hints)
        .map(|((issue, prs), hints)| {
            let this = issue.work_item_id();
            let members = prs
                .iter()
                .map(|pr| Member {
                    pr: pr.clone(),
                    status: status(observation, &lookup, &issue.owner, pr),
                    also_attached_to: attachers
                        .get(&PullRequestKey::of(&pr.repo, pr.number))
                        .filter(|_| same_login(&pr.repo.owner, &issue.owner))
                        .into_iter()
                        .flatten()
                        .filter(|g| g.issue != this)
                        .cloned()
                        .collect(),
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

    let mut pull_request_groups: HashMap<PullRequestKey, Landing> = HashMap::new();
    for (pr, possibly_for) in observation.pull_requests.iter().zip(pull_request_hints) {
        let key = PullRequestKey::of(&pr.repo, pr.number);
        let landing = Landing {
            groups: attachers.get(&key).cloned().unwrap_or_default(),
            possibly_for,
            ..Landing::default()
        };
        if !landing.is_empty() {
            // The first one read wins, as for Linear Issues.
            pull_request_groups.entry(key).or_insert(landing);
        }
    }

    Correlation {
        issues: issue_groups,
        pull_requests: pull_request_groups,
    }
}

/// The hint rule, in one place: `pr` is an open Pull Request of `issue`'s
/// Owner, not attached to it (`attached`), whose head branch names the
/// issue's identifier. `branch` and `identifier` come lowercased.
fn is_hint(
    issue: &LinearIssue,
    attached: &[PullRequestRef],
    pr: &PullRequest,
    branch: &str,
    identifier: &str,
) -> bool {
    same_login(&pr.repo.owner, &issue.owner)
        && names_identifier(branch, identifier)
        && !attached.iter().any(|p| p.is(&pr.repo, pr.number))
}

/// Each Linear Issue once, by identity (the first one read wins).
fn distinct_issues(issues: &[LinearIssue]) -> Vec<&LinearIssue> {
    let mut seen: HashSet<&LinearIssueId> = HashSet::new();
    issues
        .iter()
        .filter(|issue| seen.insert(&issue.id))
        .collect()
}

/// The Pull Requests attached to `issue`, each once (`Attachments` holds
/// no repeats).
fn attached_pull_requests(issue: &LinearIssue) -> Vec<PullRequestRef> {
    issue.attachments.pull_requests().cloned().collect()
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
fn status(
    observation: &Observation,
    lookup: &Lookup<'_>,
    owner: &str,
    pr: &PullRequestRef,
) -> MemberStatus {
    if !same_login(&pr.repo.owner, owner) {
        return MemberStatus::NotSurveyed(NotSurveyed::OtherOwner);
    }
    if let Some(found) = lookup.open.get(&PullRequestKey::of(&pr.repo, pr.number)) {
        return MemberStatus::Open {
            draft: found.is_draft,
        };
    }
    if !lookup.repos.contains(&pr.repo.folded()) {
        return MemberStatus::NotSurveyed(NotSurveyed::RepoNotSurveyed);
    }
    // The cache merge's rule: a member missing from a Repo read completely
    // is gone from its open Pull Requests; how it ended is read by number.
    if !observation
        .coverage
        .read_completely(&pr.repo, SourceKind::PullRequests)
    {
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
        // Reopened since the survey listed the Repo's open Pull Requests.
        Some(PullRequestState::Open) => MemberStatus::OpenAgain,
        // Not read yet: den can't say how it ended.
        None => MemberStatus::NotOpen,
    }
}

/// Whether `branch` contains `identifier` on word boundaries, ignoring case
/// (the caller lowercases both, ASCII only, which keeps every byte offset):
/// the characters on either side, if any, are neither letters nor digits.
/// `team-1` is in `me/team-1-fix` and `team-1`, never in `team-12-fix` or
/// `steam-1`.
fn names_identifier(branch: &str, identifier: &str) -> bool {
    if identifier.is_empty() {
        return false;
    }
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    branch.match_indices(&identifier).any(|(start, found)| {
        let before = branch.get(..start).and_then(|s| s.chars().next_back());
        let after = branch
            .get(start + found.len()..)
            .and_then(|s| s.chars().next());
        boundary(before) && boundary(after)
    })
}
