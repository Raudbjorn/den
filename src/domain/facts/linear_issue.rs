//! Linear Issue facts: what the Linear source observed about one Linear Issue
//! of an Owner with a Linear Workflow (spec 0002, "Facts: Linear Issue").
//! Nothing decided. Its attachments are reduced to Pull Request identities
//! (Owner, Repo, number) or kept as unrecognized links; which of them make a
//! Landing Group is the decision engine's (`decide::correlation`, #35).

use crate::domain::{
    coverage::LinearSourceKind,
    ids::{LinearIssueId, RepoId, WorkItemId, is_valid_login, is_valid_repo_name},
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearIssue {
    /// The Owner whose Linear Workflow surveyed it.
    pub owner: String,
    /// The identity: workspace + Linear's permanent ID.
    pub id: LinearIssueId,
    /// The Linear source whose list it came from (assigned issues, or a start
    /// team's). The cache keeps or replaces it with that source: a list read
    /// completely drops an issue it no longer has, any other keeps it (#34).
    pub listed_by: LinearSourceKind,
    /// `TEAM-123`: an attribute, never the identity.
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub team: LinearTeam,
    pub state: LinearState,
    /// `None` when nobody is assigned.
    pub assignee: Option<LinearUser>,
    pub priority: LinearPriority,
    /// `None` when the issue is in no cycle.
    pub cycle: Option<LinearCycle>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Linear's suggested branch name, unchecked (Start, #38, checks it
    /// against den's safe-name rules before using it).
    pub branch_name: Option<String>,
    /// What Linear recorded as attached to it, and whether den read it all
    /// (#35).
    pub attachments: Attachments,
}

impl LinearIssue {
    pub fn work_item_id(&self) -> WorkItemId {
        WorkItemId::LinearIssue {
            owner: self.owner.clone(),
            issue: self.id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearTeam {
    pub id: String,
    /// The team key that prefixes identifiers, e.g. `TEAM`.
    pub key: String,
}

/// A Linear workflow state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearState {
    pub id: String,
    pub name: String,
    pub kind: LinearStateKind,
}

/// Linear's workflow state type: every team's states fall in one of these.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LinearStateKind {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
    /// A type den doesn't know, as Linear named it. Never treated as any of
    /// the above.
    Other(String),
}

impl LinearStateKind {
    /// From Linear's `WorkflowState.type`.
    pub fn from_linear(kind: &str) -> Self {
        match kind {
            "triage" => Self::Triage,
            "backlog" => Self::Backlog,
            "unstarted" => Self::Unstarted,
            "started" => Self::Started,
            "completed" => Self::Completed,
            "canceled" => Self::Canceled,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Triage => "triage",
            Self::Backlog => "backlog",
            Self::Unstarted => "unstarted",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
            Self::Other(kind) => kind,
        }
    }
}

/// A Linear user. Never matched to a GitHub login (spec 0002, "Waiting On").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearUser {
    pub id: String,
    pub name: String,
}

/// Linear's priority, from its number (0 none, 1 urgent … 4 low).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LinearPriority {
    None,
    Urgent,
    High,
    Medium,
    Low,
    /// A number den doesn't know.
    Other(i64),
}

impl LinearPriority {
    pub fn from_linear(priority: i64) -> Self {
        match priority {
            0 => Self::None,
            1 => Self::Urgent,
            2 => Self::High,
            3 => Self::Medium,
            4 => Self::Low,
            other => Self::Other(other),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::None => "no priority".to_owned(),
            Self::Urgent => "urgent".to_owned(),
            Self::High => "high".to_owned(),
            Self::Medium => "medium".to_owned(),
            Self::Low => "low".to_owned(),
            Self::Other(n) => format!("priority {n}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearCycle {
    pub id: String,
    pub number: u64,
    /// Linear's optional cycle name.
    pub name: Option<String>,
    /// The team's current cycle.
    pub active: bool,
}

impl LinearCycle {
    /// "cycle 12 (Sprint 4), active".
    pub fn describe(&self) -> String {
        let name = self
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        let active = if self.active { ", active" } else { "" };
        format!("cycle {}{name}{active}", self.number)
    }
}

/// A Linear Issue's attachments, as den read them (#35).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachments {
    /// In Linear's order, without repeats ([`Attachment::same`]; the
    /// constructors and [`Attachments::push`] drop them).
    pub links: Vec<Attachment>,
    /// Whether `links` is every attachment the issue has.
    pub read: AttachmentsRead,
}

impl Attachments {
    /// Every attachment was read: `links` is all of them.
    pub fn complete(links: Vec<Attachment>) -> Self {
        Self::read_as(links, AttachmentsRead::Complete)
    }

    /// Some or all attachments are missing, for `why`.
    pub fn incomplete(links: Vec<Attachment>, why: impl Into<String>) -> Self {
        Self::read_as(links, AttachmentsRead::Incomplete { why: why.into() })
    }

    fn read_as(links: Vec<Attachment>, read: AttachmentsRead) -> Self {
        let mut attachments = Self {
            links: Vec::with_capacity(links.len()),
            read,
        };
        for link in links {
            attachments.push(link);
        }
        attachments
    }

    /// Adds `attachment` after the others, unless it repeats one of them.
    pub fn push(&mut self, attachment: Attachment) {
        if !self.links.iter().any(|seen| seen.same(&attachment)) {
            self.links.push(attachment);
        }
    }

    /// The Pull Requests among them.
    pub fn pull_requests(&self) -> impl Iterator<Item = &PullRequestRef> {
        self.links.iter().filter_map(|link| match link {
            Attachment::PullRequest(pr) => Some(pr),
            Attachment::Link { .. } => None,
        })
    }
}

/// Whether den read every attachment of a Linear Issue. Coverage per issue:
/// the source that listed the issue may be complete while one issue's
/// attachments are not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentsRead {
    Complete,
    /// Not all of them, for `why` ("Linear returned errors alongside this
    /// page", "more than 50 attachments").
    Incomplete {
        why: String,
    },
}

/// One attachment of a Linear Issue. Raw metadata isn't carried: its keys
/// depend on the integration that wrote it (spec 0002, "Facts: Linear Issue").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attachment {
    /// Its URL is a GitHub Pull Request's.
    PullRequest(PullRequestRef),
    /// Anything else, kept as Linear gave it.
    Link { url: String },
}

impl Attachment {
    /// The same attachment: two Pull Requests compared as GitHub compares
    /// them (Owner and Repo case-insensitively, see [`PullRequestRef::is`]),
    /// two links exactly.
    pub fn same(&self, other: &Attachment) -> bool {
        match (self, other) {
            (Self::PullRequest(a), Self::PullRequest(b)) => a.is(&b.repo, b.number),
            (Self::Link { url: a }, Self::Link { url: b }) => a == b,
            _ => false,
        }
    }

    /// A Pull Request when `url` parses as one, otherwise a link.
    pub fn from_url(url: &str) -> Self {
        PullRequestRef::from_url(url).map_or_else(
            || Self::Link {
                url: url.to_owned(),
            },
            Self::PullRequest,
        )
    }
}

/// A fully qualified Pull Request identity (Owner, Repo, number), as an
/// attachment names it. Two Owners' `api#42` are different Pull Requests.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PullRequestRef {
    pub repo: RepoId,
    pub number: u64,
}

/// GitHub's web host; Pull Request URLs on any other host are links.
const GITHUB_HOSTS: [&str; 2] = ["github.com", "www.github.com"];

impl PullRequestRef {
    pub fn new(repo: RepoId, number: u64) -> Self {
        Self { repo, number }
    }

    pub fn id(&self) -> WorkItemId {
        WorkItemId::PullRequest {
            repo: self.repo.clone(),
            number: self.number,
        }
    }

    /// Whether this names the Pull Request `repo` `#number` (Owner and Repo
    /// compared as GitHub does, case-insensitively).
    pub fn is(&self, repo: &RepoId, number: u64) -> bool {
        self.number == number && self.repo.same(repo)
    }

    /// `https://github.com/<owner>/<repo>/pull/<number>`, optionally followed
    /// by a sub-page (`/files`), a query or a fragment. Lenient about form on
    /// purpose, as people paste links: `http://` and `www.github.com` too,
    /// in any casing. Strict about names: anything else, or any part that
    /// isn't a valid GitHub login, Repo name or canonical Pull Request
    /// number, is `None`: den never guesses a membership from a URL it
    /// doesn't fully understand.
    pub fn from_url(url: &str) -> Option<Self> {
        let rest = strip_prefix_ignoring_case(url, "https://")
            .or_else(|| strip_prefix_ignoring_case(url, "http://"))?;
        let (host, path) = rest.split_once('/')?;
        if !GITHUB_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h)) {
            return None;
        }
        let path = path.split(['?', '#']).next().unwrap_or_default();
        let mut parts = path.split('/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        if parts.next()? != "pull" {
            return None;
        }
        let number = parts.next()?;
        let canonical_number = !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit())
            && !number.starts_with('0');
        if !(canonical_number && is_valid_login(owner) && is_valid_repo_name(repo)) {
            return None;
        }
        let number = number.parse::<u64>().ok()?;
        Some(Self::new(RepoId::new(owner, repo), number))
    }
}

fn strip_prefix_ignoring_case<'u>(text: &'u str, prefix: &str) -> Option<&'u str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| text.get(prefix.len()..))
        .flatten()
}
