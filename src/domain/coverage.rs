//! Coverage: how complete an Observation is, per source it tried (spec, "GitHub
//! source"). den never estimates how many items it could not see; it says which
//! source was incomplete and why.

use super::ids::{RepoId, WorkItemId, WorkItemKind, same_login};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// What a source fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SourceKind {
    /// The Owner's list of Repos (with fork / archived / default branch).
    Repos,
    PullRequests,
    /// Checks, reviews and threads of PRs already enumerated by PullRequests.
    PullRequestDetails,
    Issues,
    /// Linked PRs of Issues already enumerated by Issues.
    IssueDetails,
    Branches,
    Runs,
    /// One of the Linear sources of an Owner with a Linear Workflow (spec 0002,
    /// see [`LinearSourceKind`]).
    Linear(LinearSourceKind),
}

/// What a Linear source fetches for an Owner with a Linear Workflow. Each is
/// recorded as `Source::Owner { owner, kind: SourceKind::Linear(..) }`, keyed
/// by the Owner whose Linear Workflow reads it, so the per-Owner rules (the
/// cache's Owner restriction, Owner Scope coverage, "Nothing needs you") hold
/// for Linear exactly as for GitHub. Owners without a Linear Workflow record
/// none of them, so a missing Linear part never makes a survey incomplete.
///
/// They are not Work Item lists of the Owner's GitHub Repos: an incomplete
/// Repo list never marks them partial ([`SourceKind::WORK_ITEMS`]), and they
/// are not applicable to a GitHub Work Item ([`Coverage::applicable_to`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LinearSourceKind {
    /// Who the Linear viewer is in the workflow's workspace (#32).
    Viewer,
    /// The active Linear Issues assigned to the Linear viewer (#32).
    AssignedIssues,
    /// The unassigned Linear Issues of the start teams (#33).
    StartTeamIssues,
    /// Each start team's workflow states (#33).
    TeamStates,
}

impl LinearSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Viewer => "Linear viewer",
            Self::AssignedIssues => "assigned Linear Issues",
            Self::StartTeamIssues => "start-team Linear Issues",
            Self::TeamStates => "Linear team states",
        }
    }
}

impl SourceKind {
    /// The lists that yield Work Items, excluding Repo metadata and nested details.
    pub const WORK_ITEMS: [Self; 4] =
        [Self::PullRequests, Self::Issues, Self::Branches, Self::Runs];

    pub fn for_work_item(kind: WorkItemKind) -> Self {
        match kind {
            WorkItemKind::Issue => Self::Issues,
            WorkItemKind::PullRequest => Self::PullRequests,
            WorkItemKind::Branch => Self::Branches,
            WorkItemKind::Run => Self::Runs,
            WorkItemKind::LinearIssue => Self::Linear(LinearSourceKind::AssignedIssues),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Repos => "Repos",
            Self::PullRequests => "Pull Requests",
            Self::PullRequestDetails => "Pull Request details",
            Self::Issues => "Issues",
            Self::IssueDetails => "Issue details",
            Self::Branches => "Branches",
            Self::Runs => "Runs",
            Self::Linear(kind) => kind.label(),
        }
    }
}

/// One thing den tried to read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Source {
    /// Who the viewer is and which teams they belong to.
    Viewer,
    /// One kind of fact across all of an Owner's Repos.
    Owner { owner: String, kind: SourceKind },
    /// One kind of fact for a single Repo (recorded when that Repo needed its own
    /// follow-up queries, for example more than one page of Pull Requests).
    Repo { repo: RepoId, kind: SourceKind },
}

impl Source {
    /// The same source on GitHub, whatever the casing of its Owner and Repo.
    pub fn same(&self, other: &Source) -> bool {
        match (self, other) {
            (Self::Viewer, Self::Viewer) => true,
            (Self::Owner { owner, kind }, Self::Owner { owner: o, kind: k }) => {
                kind == k && same_login(owner, o)
            }
            (Self::Repo { repo, kind }, Self::Repo { repo: r, kind: k }) => {
                kind == k && repo.same(r)
            }
            _ => false,
        }
    }

    /// One of an Owner's Linear sources.
    pub fn is_linear(&self) -> bool {
        matches!(
            self,
            Self::Owner {
                kind: SourceKind::Linear(_),
                ..
            }
        )
    }

    pub fn label(&self) -> String {
        match self {
            Self::Viewer => "viewer and teams".to_owned(),
            Self::Owner { owner, kind } => format!("{} of {owner}", kind.label()),
            Self::Repo { repo, kind } => format!("{} of {repo}", kind.label()),
        }
    }
}

/// Why a source's data is incomplete even though some of it arrived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Incompleteness {
    /// GitHub's GraphQL response carried errors next to data.
    ErrorsAlongsideData { errors: Vec<String> },
    /// GitHub's rate limit was hit; `resets_at` when GitHub said.
    RateLimited { resets_at: Option<Timestamp> },
    /// Pagination stopped before the last page (den's page bound, or a failed page).
    PaginationStopped { pages_read: u32, reason: String },
    /// The list was read, but a follow-up query for some of its items' facts
    /// was lost (for example a PR's checks and reviews).
    FollowUpLost { what: String, reason: String },
    /// A follow-up query was read, but some of its lists were not read in
    /// full: GitHub had more than den reads in one query, or some entries
    /// came back unreadable (for example a PR's checks or review threads).
    FollowUpTruncated { what: String },
    /// The Owner's Repo list was not read completely, so Work Items in the
    /// Repos den missed may be missing too (whatever this kind's own fetch said).
    RepoListIncomplete { owner: String },
    /// Linear's GraphQL responses carried errors next to data: the first
    /// [`MAX_KEPT_LINEAR_ERRORS`] messages, and how many more there were
    /// (counted, not kept, so one reply can't grow coverage, the cache or
    /// the log without bound).
    LinearErrorsAlongsideData {
        errors: Vec<String>,
        #[serde(default)]
        more: usize,
    },
    /// Linear's rate limit was hit; `resets_at` when Linear said.
    LinearRateLimited { resets_at: Option<Timestamp> },
}

/// The most Linear error messages one source keeps; the rest are counted.
pub const MAX_KEPT_LINEAR_ERRORS: usize = 5;

impl Incompleteness {
    /// Linear's errors alongside data: the first [`MAX_KEPT_LINEAR_ERRORS`]
    /// of `messages`, the rest counted with `more` (messages already dropped).
    pub fn linear_errors(messages: impl IntoIterator<Item = String>, more: usize) -> Self {
        let mut errors = Vec::new();
        let mut dropped = more;
        for message in messages {
            if errors.len() < MAX_KEPT_LINEAR_ERRORS {
                errors.push(message);
            } else {
                dropped = dropped.saturating_add(1);
            }
        }
        Self::LinearErrorsAlongsideData {
            errors,
            more: dropped,
        }
    }

    /// Ranks reasons so that combining two keeps the more telling one: a rate
    /// limit (which also stops pagination, and says when it resets) over whole
    /// items missing (a stopped pagination, or a Repo list read only in part)
    /// over facts missing from items den has (errors alongside data, a lost or
    /// truncated follow-up query).
    fn rank(&self) -> u8 {
        match self {
            Self::ErrorsAlongsideData { .. }
            | Self::LinearErrorsAlongsideData { .. }
            | Self::FollowUpLost { .. }
            | Self::FollowUpTruncated { .. } => 0,
            Self::PaginationStopped { .. } | Self::RepoListIncomplete { .. } => 1,
            Self::RateLimited { .. } | Self::LinearRateLimited { .. } => 2,
        }
    }

    /// Combines two reasons for one source. Errors alongside data from several
    /// responses are all kept; of two rate limits the later known reset is kept;
    /// otherwise the higher-ranked reason wins (the first one wins a tie).
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (
                Self::ErrorsAlongsideData { mut errors },
                Self::ErrorsAlongsideData { errors: more },
            ) => {
                errors.extend(more);
                Self::ErrorsAlongsideData { errors }
            }
            (Self::RateLimited { resets_at: first }, Self::RateLimited { resets_at: second }) => {
                Self::RateLimited {
                    resets_at: first.max(second),
                }
            }
            (
                Self::LinearErrorsAlongsideData { errors, more },
                Self::LinearErrorsAlongsideData {
                    errors: next,
                    more: next_more,
                },
            ) => Self::linear_errors(
                errors.into_iter().chain(next),
                more.saturating_add(next_more),
            ),
            (
                Self::LinearRateLimited { resets_at: first },
                Self::LinearRateLimited { resets_at: second },
            ) => Self::LinearRateLimited {
                resets_at: first.max(second),
            },
            (first, second) => {
                if second.rank() > first.rank() {
                    second
                } else {
                    first
                }
            }
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::ErrorsAlongsideData { errors } => match errors.as_slice() {
                [] => "GitHub returned errors alongside data".to_owned(),
                [only] => format!("GitHub returned an error alongside data: {only}"),
                [first, rest @ ..] => format!(
                    "GitHub returned {} errors alongside data, first: {first}",
                    rest.len() + 1
                ),
            },
            Self::RateLimited {
                resets_at: Some(at),
            } => {
                format!("rate limited by GitHub until {at}")
            }
            Self::RateLimited { resets_at: None } => "rate limited by GitHub".to_owned(),
            Self::PaginationStopped { pages_read, reason } => {
                format!("stopped after {pages_read} page(s): {reason}")
            }
            Self::FollowUpLost { what, reason } => format!("{what} could not be read: {reason}"),
            Self::FollowUpTruncated { what } => format!("not every entry was read: {what}"),
            Self::RepoListIncomplete { owner } => {
                format!(
                    "the Repo list of {owner} was not read completely, so some of its Repos may be missing"
                )
            }
            Self::LinearErrorsAlongsideData { errors, more } => {
                match (errors.as_slice(), errors.len().saturating_add(*more)) {
                    ([], _) => "Linear returned errors alongside data".to_owned(),
                    ([only], 1) => format!("Linear returned an error alongside data: {only}"),
                    ([first, ..], total) => {
                        format!("Linear returned {total} errors alongside data, first: {first}")
                    }
                }
            }
            Self::LinearRateLimited {
                resets_at: Some(at),
            } => format!("rate limited by Linear until {at}"),
            Self::LinearRateLimited { resets_at: None } => "rate limited by Linear".to_owned(),
        }
    }

    /// When a rate limit resets: `Some(None)` for a rate limit that didn't
    /// say, `None` for anything that isn't a rate limit.
    pub fn rate_limit_reset(&self) -> Option<Option<Timestamp>> {
        match self {
            Self::RateLimited { resets_at } | Self::LinearRateLimited { resets_at } => {
                Some(*resets_at)
            }
            _ => None,
        }
    }
}

/// How completely one source was read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceState {
    Complete,
    Partial(Incompleteness),
    Failed { error: String },
    NotYetFetched,
}

impl SourceState {
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }

    /// Ranks states so that combining two keeps the worse one.
    fn severity(&self) -> u8 {
        match self {
            Self::Complete => 0,
            Self::Partial(_) => 1,
            Self::NotYetFetched => 2,
            Self::Failed { .. } => 3,
        }
    }

    /// The worse of two states (the first one wins a tie). Two partial states
    /// combine their reasons (see [`Incompleteness`]), so a later rate limit is
    /// not lost behind an earlier page's errors.
    pub fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::Partial(first), Self::Partial(second)) => Self::Partial(first.combine(second)),
            (first, second) => {
                if second.severity() > first.severity() {
                    second
                } else {
                    first
                }
            }
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Complete => "complete".to_owned(),
            Self::Partial(why) => format!("partial: {}", why.describe()),
            Self::Failed { error } => format!("failed: {error}"),
            Self::NotYetFetched => "not fetched yet".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageEntry {
    pub source: Source,
    pub state: SourceState,
}

/// Coverage for every source an Observation tried.
///
/// Invariant: an Owner's Work Item kinds are never recorded as more complete
/// than its Repo list. Recording an incomplete `Owner { kind: Repos }` also
/// records every Work Item kind of that Owner as partial
/// ([`Incompleteness::RepoListIncomplete`]), or not yet fetched when the Repo
/// list was not fetched yet. This holds whichever order the sources are
/// recorded in, so the header, the sidebar, each item's applicable coverage,
/// the cache merge and "Nothing needs you" all see it without knowing the rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    entries: Vec<CoverageEntry>,
}

impl Coverage {
    /// Records a source's state. Recording the same source again keeps the worse
    /// state. An incomplete Repo list also marks the Owner's Work Item kinds
    /// (see the type's invariant).
    pub fn record(&mut self, source: Source, state: SourceState) {
        if let Source::Owner {
            owner,
            kind: SourceKind::Repos,
        } = &source
            && !state.is_complete()
        {
            let derived = match &state {
                SourceState::NotYetFetched => SourceState::NotYetFetched,
                _ => SourceState::Partial(Incompleteness::RepoListIncomplete {
                    owner: owner.clone(),
                }),
            };
            for kind in SourceKind::WORK_ITEMS {
                self.record_one(
                    Source::Owner {
                        owner: owner.clone(),
                        kind,
                    },
                    derived.clone(),
                );
            }
        }
        self.record_one(source, state);
    }

    fn record_one(&mut self, source: Source, state: SourceState) {
        match self.entries.iter_mut().find(|e| e.source == source) {
            Some(existing) => {
                let previous = std::mem::replace(&mut existing.state, SourceState::Complete);
                // A source's own reason for being partial says more than the one
                // derived from its Owner's Repo list, so it replaces it.
                let derived = matches!(
                    previous,
                    SourceState::Partial(Incompleteness::RepoListIncomplete { .. })
                );
                existing.state = if derived && matches!(state, SourceState::Partial(_)) {
                    state
                } else {
                    previous.worst(state)
                };
            }
            None => self.entries.push(CoverageEntry { source, state }),
        }
    }

    pub fn entries(&self) -> &[CoverageEntry] {
        &self.entries
    }

    pub fn get(&self, source: &Source) -> Option<&SourceState> {
        self.entries
            .iter()
            .find(|e| &e.source == source)
            .map(|e| &e.state)
    }

    /// True when every source den tried was read completely.
    pub fn is_complete(&self) -> bool {
        self.entries.iter().all(|e| e.state.is_complete())
    }

    /// The entries that are not complete.
    pub fn incomplete(&self) -> impl Iterator<Item = &CoverageEntry> {
        self.entries.iter().filter(|e| !e.state.is_complete())
    }

    /// Keeps only the entries `keep` accepts (no cross-kind rule applies:
    /// dropping an entry never changes another).
    pub fn retain(&mut self, keep: impl FnMut(&CoverageEntry) -> bool) {
        self.entries.retain(keep);
    }

    /// Whether den read every `kind` fact of `repo`: the one rule for "is a
    /// fact of this Repo that den didn't see really gone?", shared by the
    /// cache merge and the Landing Groups' "merged or closed".
    ///
    /// A Repo with a source of its own is judged by that source alone: read
    /// completely, its facts are all there even if a later page of its
    /// Owner's list was lost. A Repo without one was read (or not) as part of
    /// its Owner's list, so only a complete Owner source says it is all
    /// there.
    ///
    /// Matched case-insensitively, as GitHub names are: entries under two
    /// spellings are one source, read completely when any of them was (a
    /// complete read of a Repo holds all its facts, whatever another
    /// spelling's entry says).
    pub fn read_completely(&self, repo: &RepoId, kind: SourceKind) -> bool {
        let own: Vec<&CoverageEntry> = self
            .entries
            .iter()
            .filter(|e| match &e.source {
                Source::Repo { repo: r, kind: k } => *k == kind && r.same(repo),
                _ => false,
            })
            .collect();
        if !own.is_empty() {
            return own.iter().any(|e| e.state.is_complete());
        }
        self.entries.iter().any(|e| match &e.source {
            Source::Owner { owner, kind: k } => {
                *k == kind && same_login(owner, &repo.owner) && e.state.is_complete()
            }
            _ => false,
        })
    }

    /// The sources whose completeness bears on one Work Item: the viewer (for
    /// Waiting On), its kind across its Owner, and its kind for its own Repo.
    /// Matched case-insensitively, as GitHub names are: a fact kept from an
    /// earlier survey may spell its Owner as the config did then.
    ///
    /// A Linear Issue's are its Owner's Linear sources: the Linear viewer (for
    /// Waiting On) and the issue lists. The GitHub viewer doesn't bear on it.
    pub fn applicable_to(&self, item: &WorkItemId) -> Vec<CoverageEntry> {
        let Some(repo) = item.repo() else {
            return self
                .entries
                .iter()
                .filter(|e| match &e.source {
                    Source::Owner {
                        owner,
                        kind: SourceKind::Linear(k),
                    } => {
                        same_login(owner, item.owner())
                            && matches!(
                                k,
                                LinearSourceKind::Viewer
                                    | LinearSourceKind::AssignedIssues
                                    | LinearSourceKind::StartTeamIssues
                            )
                    }
                    _ => false,
                })
                .cloned()
                .collect();
        };
        let kind = SourceKind::for_work_item(item.kind());
        let relevant = |k: SourceKind| {
            k == kind
                || matches!(
                    (kind, k),
                    (SourceKind::PullRequests, SourceKind::PullRequestDetails)
                        | (SourceKind::Issues, SourceKind::IssueDetails)
                )
        };
        self.entries
            .iter()
            .filter(|e| match &e.source {
                Source::Viewer => true,
                Source::Owner { owner, kind: k } => relevant(*k) && same_login(owner, &repo.owner),
                Source::Repo { repo: r, kind: k } => relevant(*k) && r.same(repo),
            })
            .cloned()
            .collect()
    }
}
