//! Ordering and Scope (pure): which Decisions the list shows, and in what order
//! (spec, "Ordering").
//!
//! Order: Waiting On (me › nobody › someone else), then the kind order (Fix CI
//! on a default branch › Resolve conflicts › Address review › Land › Merge ›
//! Review › Request review › Fix CI on a PR › Finish draft › Mark for release ›
//! Implement › Start › Triage › Open PR › Prune or revive › no Next Step; spec
//! 0002, "Ordering"), then most recently updated first, except that Starts
//! order by priority, then active cycle, then oldest first
//! ([`StartOrder`](crate::decide::StartOrder)). The Work Item's
//! identity breaks remaining ties, so the order is total and stable across
//! refreshes.

use crate::{
    decide::Decision,
    domain::{NextStep, RepoId, WorkItemId, WorkItemKind, ids::same_login},
};
use std::{cmp::Reverse, fmt};

/// What the Next Step list covers (CONTEXT.md). Boards are milestone 2.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Scope {
    #[default]
    AllOwners,
    Owner(String),
    Repo(RepoId),
}

impl Scope {
    /// Whether a Work Item belongs to this Scope.
    ///
    /// A Linear Issue belongs to its Owner, so the Owner and All Owners
    /// Scopes include it. A Repo Scope will include the Linear Issues with a
    /// member PR there (#35); before Landing Groups none has one.
    pub fn contains(&self, item: &WorkItemId) -> bool {
        match item.repo() {
            Some(repo) => self.contains_repo(repo),
            None => match self {
                Self::AllOwners => true,
                Self::Owner(owner) => same_login(owner, item.owner()),
                Self::Repo(_) => false,
            },
        }
    }

    /// Whether a Repo belongs to this Scope.
    pub fn contains_repo(&self, repo: &RepoId) -> bool {
        match self {
            Self::AllOwners => true,
            Self::Owner(owner) => same_login(owner, &repo.owner),
            Self::Repo(scoped) => same_repo(scoped, repo),
        }
    }

    /// Whether two Scopes name the same thing. GitHub logins and Repo names are
    /// case-insensitive, so a refresh that returns a different casing keeps the
    /// sidebar cursor and the current Scope on their row.
    pub fn same(&self, other: &Scope) -> bool {
        match (self, other) {
            (Self::AllOwners, Self::AllOwners) => true,
            (Self::Owner(a), Self::Owner(b)) => same_login(a, b),
            (Self::Repo(a), Self::Repo(b)) => same_repo(a, b),
            _ => false,
        }
    }

    /// The Owner this Scope is narrowed to, if any (a Repo's Owner for a Repo).
    pub fn owner(&self) -> Option<&str> {
        match self {
            Self::AllOwners => None,
            Self::Owner(owner) => Some(owner),
            Self::Repo(repo) => Some(&repo.owner),
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllOwners => f.write_str("All Owners"),
            Self::Owner(owner) => write!(f, "Owner {owner}"),
            Self::Repo(repo) => write!(f, "Repo {repo}"),
        }
    }
}

/// GitHub owner and repository names are case-insensitive.
pub fn same_repo(a: &RepoId, b: &RepoId) -> bool {
    same_login(&a.owner, &b.owner) && a.name.eq_ignore_ascii_case(&b.name)
}

/// Whether two ids name the same Work Item: the Repo compared as GitHub does
/// (case-insensitively), the rest exactly (Branch names are case-sensitive).
pub fn same_work_item(a: &WorkItemId, b: &WorkItemId) -> bool {
    let rest_matches = match (a, b) {
        (WorkItemId::Issue { number: x, .. }, WorkItemId::Issue { number: y, .. })
        | (WorkItemId::PullRequest { number: x, .. }, WorkItemId::PullRequest { number: y, .. }) => {
            x == y
        }
        (WorkItemId::Branch { name: x, .. }, WorkItemId::Branch { name: y, .. }) => x == y,
        (WorkItemId::Run { workflow: x, .. }, WorkItemId::Run { workflow: y, .. }) => x == y,
        // Workspace + Linear's ID; the Owner folds like a login.
        (
            WorkItemId::LinearIssue {
                owner: x_owner,
                issue: x,
            },
            WorkItemId::LinearIssue {
                owner: y_owner,
                issue: y,
            },
        ) => return x == y && same_login(x_owner, y_owner),
        _ => false,
    };
    rest_matches
        && match (a.repo(), b.repo()) {
            (Some(x), Some(y)) => same_repo(x, y),
            _ => false,
        }
}

/// Where a Decision's Next Step falls in the kind order; lower comes first.
/// Fix CI ranks by where it failed: a workflow on a default branch (a Run) is
/// the most urgent kind, a Pull Request's checks come after Review. Items with
/// no Next Step come last.
pub fn kind_rank(next_step: Option<NextStep>, kind: WorkItemKind) -> u8 {
    let Some(step) = next_step else {
        return NO_STEP_RANK;
    };
    match (step, kind) {
        (NextStep::FixCi, WorkItemKind::Run) => 0,
        (NextStep::ResolveConflicts, _) => 1,
        (NextStep::AddressReview, _) => 2,
        (NextStep::Land, _) => 3,
        (NextStep::Merge, _) => 4,
        (NextStep::Review, _) => 5,
        (NextStep::RequestReview, _) => 6,
        (NextStep::FixCi, _) => 7,
        (NextStep::FinishDraft, _) => 8,
        (NextStep::MarkForRelease, _) => 9,
        (NextStep::Implement, _) => 10,
        (NextStep::Start, _) => 11,
        (NextStep::Triage, _) => 12,
        (NextStep::OpenPr, _) => 13,
        (NextStep::PruneOrRevive, _) => 14,
    }
}

/// The rank of a Work Item with no Next Step: after every kind.
const NO_STEP_RANK: u8 = 15;

/// Sorts Decisions for display (see the module docs).
pub fn order(mut decisions: Vec<Decision>) -> Vec<Decision> {
    decisions.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    decisions
}

fn sort_key(d: &Decision) -> impl Ord + '_ {
    (
        d.waiting_on,
        kind_rank(d.next_step, d.item.id.kind()),
        // Only Starts carry one, so it orders Starts among themselves and
        // leaves every other kind to the update time.
        d.start_order,
        Reverse(d.item.updated_at),
        &d.item.id,
    )
}

/// The Decisions in `scope`, keeping their order.
pub fn in_scope<'a>(
    decisions: &'a [Decision],
    scope: &'a Scope,
) -> impl Iterator<Item = &'a Decision> + 'a {
    decisions.iter().filter(|d| scope.contains(&d.item.id))
}
