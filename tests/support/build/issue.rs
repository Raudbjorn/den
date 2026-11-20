//! Issue builder: an open Issue with no labels, no assignee and no linked PR
//! (a Triage Issue) unless told otherwise.

use super::{repo_id, ts};
use den::domain::facts::{Issue, IssueLink, LinkKind, LinkedPrState};

/// When a built Issue was last updated, unless set.
const DEFAULT_UPDATED: &str = "2026-11-17T12:00:00Z";

/// An unlabelled, unassigned, unlinked Issue in `repo` ("owner/name").
pub fn issue(repo: &str, number: u64) -> IssueBuilder {
    let repo = repo_id(repo);
    IssueBuilder {
        issue: Issue {
            url: format!("https://github.com/{repo}/issues/{number}"),
            repo,
            number,
            title: format!("Issue {number}"),
            labels: Vec::new(),
            labels_complete: true,
            assignees: Vec::new(),
            assignees_complete: true,
            updated_at: ts(DEFAULT_UPDATED),
            linked_prs: Vec::new(),
            linked_prs_complete: true,
        },
    }
}

pub struct IssueBuilder {
    issue: Issue,
}

impl IssueBuilder {
    pub fn label(mut self, name: &str) -> Self {
        self.issue.labels.push(name.to_owned());
        self
    }

    pub fn assignee(mut self, login: &str) -> Self {
        self.issue.assignees.push(login.to_owned());
        self
    }

    pub fn title(mut self, title: &str) -> Self {
        self.issue.title = title.to_owned();
        self
    }

    pub fn updated(mut self, at: &str) -> Self {
        self.issue.updated_at = ts(at);
        self
    }

    /// A PR `pr_repo#number` ("owner/name") linked by `kind`, in `state`.
    pub fn linked(
        mut self,
        kind: LinkKind,
        pr_repo: &str,
        number: u64,
        state: LinkedPrState,
    ) -> Self {
        self.issue.linked_prs.push(IssueLink {
            kind,
            repo: repo_id(pr_repo),
            number,
            state,
        });
        self
    }

    /// GitHub's labels for the Issue could not all be read.
    pub fn labels_unread(mut self) -> Self {
        self.issue.labels_complete = false;
        self
    }

    /// GitHub's assignees for the Issue could not all be read.
    pub fn assignees_unread(mut self) -> Self {
        self.issue.assignees_complete = false;
        self
    }

    /// GitHub reported more linked PRs than were read.
    pub fn more_links_unread(mut self) -> Self {
        self.issue.linked_prs_complete = false;
        self
    }

    pub fn build(self) -> Issue {
        self.issue
    }
}
