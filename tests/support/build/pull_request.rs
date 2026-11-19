//! Pull Request builder. The PR rules ticket (#4) adds setters for the facts it
//! introduces (mergeability, checks, reviews, merge queue).

use super::{repo_id, ts};
use den::domain::facts::PullRequest;

/// When a built PR was last updated, unless set.
const DEFAULT_UPDATED: &str = "2026-11-18T12:00:00Z";

/// A ready, non-draft PR in `repo` ("owner/name"), authored by "someone".
pub fn pr(repo: &str, number: u64) -> PrBuilder {
    let repo = repo_id(repo);
    PrBuilder {
        pr: PullRequest {
            url: format!("https://github.com/{repo}/pull/{number}"),
            head_repo: Some(repo.clone()),
            repo,
            number,
            title: format!("PR {number}"),
            author: Some("someone".to_owned()),
            is_draft: false,
            base_ref: "main".to_owned(),
            head_ref: format!("feature-{number}"),
            head_commit: format!("{number:040x}"),
            created_at: ts(DEFAULT_UPDATED),
            updated_at: ts(DEFAULT_UPDATED),
        },
    }
}

pub struct PrBuilder {
    pr: PullRequest,
}

impl PrBuilder {
    pub fn draft(mut self) -> Self {
        self.pr.is_draft = true;
        self
    }

    pub fn author(mut self, login: &str) -> Self {
        self.pr.author = Some(login.to_owned());
        self
    }

    /// GitHub has no author (a deleted account).
    pub fn no_author(mut self) -> Self {
        self.pr.author = None;
        self
    }

    pub fn title(mut self, title: &str) -> Self {
        self.pr.title = title.to_owned();
        self
    }

    pub fn updated(mut self, at: &str) -> Self {
        self.pr.updated_at = ts(at);
        self
    }

    pub fn build(self) -> PullRequest {
        self.pr
    }
}
