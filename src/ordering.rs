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
    /// Scopes include it; a Repo Scope never does by identity alone. Whether
    /// a Repo Scope includes one depends on its Landing Group:
    /// [`Scope::contains_decision`].
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

    /// Whether a decided Work Item belongs to this Scope: [`Scope::contains`],
    /// and a Repo Scope also includes the Linear Issues with a member (an
    /// attached Pull Request, never a hint) in that Repo (spec 0002,
    /// "Ordering and Scope"; #35). The Decision was made on the whole
    /// Observation, so narrowing never changes it.
    pub fn contains_decision(&self, decision: &Decision) -> bool {
        if self.contains(&decision.item.id) {
            return true;
        }
        match self {
            Self::Repo(scoped) => decision
                .landing
                .members
                .iter()
                .any(|member| same_repo(scoped, &member.pr.repo)),
            Self::AllOwners | Self::Owner(_) => false,
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

    /// Whether this Scope can hold Linear Issues, and so whether its Owner's
    /// Linear sources bear on its coverage (a Linear source den could not
    /// read may hide work only where Linear Issues can appear).
    ///
    /// The Owner and All Owners Scopes hold them. A Repo Scope holds none
    /// until Landing Groups (#35) put the Linear Issues with a member PR
    /// there into it; whatever makes a Repo Scope hold them must turn this
    /// on with it, for every Repo of the Owner (which Repo a Linear Issue's
    /// member is in can't be known from a Linear source den couldn't read).
    pub fn holds_linear_issues(&self) -> bool {
        match self {
            Self::AllOwners | Self::Owner(_) => true,
            Self::Repo(_) => false,
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

/// Whether two ids name the same Work Item ([`WorkItemId::same`]).
pub fn same_work_item(a: &WorkItemId, b: &WorkItemId) -> bool {
    a.same(b)
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
    decisions.iter().filter(|d| scope.contains_decision(d))
}
