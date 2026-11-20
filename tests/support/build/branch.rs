//! Branch builder: a Branch that is a Work Item (never a PR head, commits
//! ahead of its Repo's `main`) unless told otherwise.

use super::{now, repo_id, ts};
use den::domain::{
    Repo,
    facts::{Branch, ComparisonBranch},
};
use jiff::SignedDuration;

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;
/// A built Branch's last commit is this many days old, unless set.
const DEFAULT_AGE_DAYS: i64 = 5;

/// A Branch `name` in `repo` ("owner/name"), 2 commits ahead of `main`, last
/// committed by "someone" 5 days before [`super::NOW`], never a PR head.
pub fn branch(repo: &str, name: &str) -> BranchBuilder {
    let repo = repo_id(repo);
    BranchBuilder {
        branch: Branch {
            url: format!("https://github.com/{repo}/tree/{name}"),
            comparison: ComparisonBranch {
                repo: repo.clone(),
                name: "main".to_owned(),
            },
            repo,
            name: name.to_owned(),
            last_commit_at: now() - SignedDuration::from_secs(DEFAULT_AGE_DAYS * SECONDS_PER_DAY),
            last_commit_author: Some("someone".to_owned()),
            last_commit_headline: format!("work on {name}"),
            commits_ahead: Some(2),
            ever_had_pr: false,
        },
    }
}

pub struct BranchBuilder {
    branch: Branch,
}

impl BranchBuilder {
    /// The last commit was made exactly `days` days before [`super::NOW`].
    pub fn age_days(self, days: i64) -> Self {
        self.age(SignedDuration::from_secs(days * SECONDS_PER_DAY))
    }

    pub fn age(mut self, age: SignedDuration) -> Self {
        self.branch.last_commit_at = now() - age;
        self
    }

    pub fn committed(mut self, at: &str) -> Self {
        self.branch.last_commit_at = ts(at);
        self
    }

    pub fn author(mut self, login: &str) -> Self {
        self.branch.last_commit_author = Some(login.to_owned());
        self
    }

    /// GitHub could not match the last commit's author to a login.
    pub fn unmatched_author(mut self) -> Self {
        self.branch.last_commit_author = None;
        self
    }

    pub fn ahead(mut self, commits: u32) -> Self {
        self.branch.commits_ahead = Some(commits);
        self
    }

    /// den did not learn how many commits it is ahead.
    pub fn not_compared(mut self) -> Self {
        self.branch.commits_ahead = None;
        self
    }

    /// Some PR (open, merged or closed) once used it as its head.
    pub fn had_pr(mut self) -> Self {
        self.branch.ever_had_pr = true;
        self
    }

    pub fn compared_against(mut self, repo: &str, name: &str) -> Self {
        self.branch.comparison = ComparisonBranch {
            repo: repo_id(repo),
            name: name.to_owned(),
        };
        self
    }

    pub fn headline(mut self, headline: &str) -> Self {
        self.branch.last_commit_headline = headline.to_owned();
        self
    }

    pub fn build(self) -> Branch {
        self.branch
    }
}

/// A fork `full_name` of `parent` (whose default branch is `main`), with its
/// own default branch.
pub fn fork(full_name: &str, parent: &str, default_branch: &str) -> Repo {
    let id = repo_id(full_name);
    Repo {
        url: format!("https://github.com/{id}"),
        id,
        is_fork: true,
        is_archived: false,
        default_branch: Some(default_branch.to_owned()),
        parent: Some(Box::new(super::upstream_repo(parent))),
    }
}
