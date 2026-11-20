//! Upstream as a target (ADR-0003): forks are standalone unless den has been
//! told, per fork, to target the fork's Upstream instead (`u` in the sidebar).
//! That choice is den's own local setting, kept in its state directory; this
//! module is the pure part: the set of chosen forks, and what a Repo's Branches
//! are compared against given that set.

use super::{
    facts::ComparisonBranch,
    ids::{RepoId, same_login},
    observation::Repo,
};
use serde::{Deserialize, Serialize};

/// The forks whose target is their Upstream. Empty by default: every fork is
/// its own target. Repos are matched case-insensitively, as GitHub does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UpstreamTargets {
    forks: Vec<RepoId>,
}

impl UpstreamTargets {
    pub fn new(forks: impl IntoIterator<Item = RepoId>) -> Self {
        let mut targets = Self::default();
        for fork in forks {
            if !targets.contains(&fork) {
                targets.forks.push(fork);
            }
        }
        targets.forks.sort();
        targets
    }

    /// Whether `fork`'s target is its Upstream.
    pub fn contains(&self, fork: &RepoId) -> bool {
        self.forks.iter().any(|f| same(f, fork))
    }

    /// Flips `fork` between targeting itself and its Upstream; returns whether
    /// its target is now its Upstream.
    pub fn toggle(&mut self, fork: &RepoId) -> bool {
        if self.contains(fork) {
            self.forks.retain(|f| !same(f, fork));
            false
        } else {
            self.forks.push(fork.clone());
            self.forks.sort();
            true
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &RepoId> {
        self.forks.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.forks.is_empty()
    }
}

fn same(a: &RepoId, b: &RepoId) -> bool {
    same_login(&a.owner, &b.owner) && a.name.eq_ignore_ascii_case(&b.name)
}

/// What a Repo's Branches should be compared against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    /// This branch: the Repo's own default branch, or its Upstream's when the
    /// Upstream is the fork's target.
    Against(ComparisonBranch),
    /// The Repo has no default branch (it is empty), so nothing to compare.
    Nothing,
    /// The Upstream is the target, but den can't see the Upstream or its
    /// default branch, so the comparison the user asked for can't be made.
    UpstreamUnavailable { fork: RepoId },
}

/// The comparison branch for `repo`'s Branches given the Upstream choices: the
/// Repo's own default branch (for a fork, the fork's: ADR-0003), or, when the
/// fork's target is its Upstream, the Upstream's default branch.
pub fn comparison_for(repo: &Repo, upstreams: &UpstreamTargets) -> Comparison {
    if repo.is_fork && upstreams.contains(&repo.id) {
        return match repo
            .upstream()
            .and_then(|up| Some((up, up.default_branch.as_ref()?)))
        {
            Some((upstream, name)) => Comparison::Against(ComparisonBranch {
                repo: upstream.id.clone(),
                name: name.clone(),
            }),
            None => Comparison::UpstreamUnavailable {
                fork: repo.id.clone(),
            },
        };
    }
    match &repo.default_branch {
        Some(name) => Comparison::Against(ComparisonBranch {
            repo: repo.id.clone(),
            name: name.clone(),
        }),
        None => Comparison::Nothing,
    }
}
