//! Linear Issue builder: an issue in workspace `acme`'s team `TEAM`, in a
//! started state, assigned to the Linear viewer [`LINEAR_ME_ID`], unless told
//! otherwise; and the Linear viewer itself.

use super::ts;
use den::domain::{
    LinearIssueId, LinearSourceKind, LinearViewer,
    facts::{
        LinearCycle, LinearIssue, LinearPriority, LinearState, LinearStateKind, LinearTeam,
        LinearUser,
    },
};

/// The Linear viewer's permanent user ID in every test workspace.
pub const LINEAR_ME_ID: &str = "user-me";
/// The Linear viewer's display name.
pub const LINEAR_ME_NAME: &str = "Me Myself";
/// When a built Linear Issue was last updated, unless set.
const DEFAULT_UPDATED: &str = "2026-11-17T12:00:00Z";

/// The Linear viewer of `owner`'s Linear Workflow on `workspace`.
pub fn linear_viewer(owner: &str, workspace: &str) -> LinearViewer {
    LinearViewer {
        owner: owner.to_owned(),
        workspace: workspace.to_owned(),
        id: LINEAR_ME_ID.to_owned(),
        name: LINEAR_ME_NAME.to_owned(),
    }
}

/// A Linear Issue of `owner` in workspace `acme`, with Linear's permanent ID
/// `id` and identifier `identifier` (`TEAM-123`).
pub fn linear_issue(owner: &str, id: &str, identifier: &str) -> LinearIssueBuilder {
    let team_key = identifier.split('-').next().unwrap_or("TEAM").to_owned();
    LinearIssueBuilder {
        issue: LinearIssue {
            owner: owner.to_owned(),
            id: LinearIssueId::new("acme", id),
            listed_by: LinearSourceKind::AssignedIssues,
            identifier: identifier.to_owned(),
            title: format!("Linear {identifier}"),
            url: format!(
                "https://linear.app/acme/issue/{}",
                identifier.to_lowercase()
            ),
            team: LinearTeam {
                id: format!("team-{}", team_key.to_lowercase()),
                key: team_key,
            },
            state: LinearState {
                id: "state-started".to_owned(),
                name: "In Progress".to_owned(),
                kind: LinearStateKind::Started,
            },
            assignee: Some(LinearUser {
                id: LINEAR_ME_ID.to_owned(),
                name: LINEAR_ME_NAME.to_owned(),
            }),
            priority: LinearPriority::High,
            cycle: None,
            created_at: ts("2026-11-01T12:00:00Z"),
            updated_at: ts(DEFAULT_UPDATED),
            branch_name: None,
        },
    }
}

pub struct LinearIssueBuilder {
    issue: LinearIssue,
}

impl LinearIssueBuilder {
    pub fn workspace(mut self, workspace: &str) -> Self {
        self.issue.id.workspace = workspace.to_owned();
        self
    }

    pub fn title(mut self, title: &str) -> Self {
        self.issue.title = title.to_owned();
        self
    }

    /// Assigned to another Linear user (`id`, `name`).
    pub fn assignee(mut self, id: &str, name: &str) -> Self {
        self.issue.assignee = Some(LinearUser {
            id: id.to_owned(),
            name: name.to_owned(),
        });
        self
    }

    /// Listed by another Linear source than the assigned issues.
    pub fn listed_by(mut self, source: LinearSourceKind) -> Self {
        self.issue.listed_by = source;
        self
    }

    pub fn unassigned(mut self) -> Self {
        self.issue.assignee = None;
        self
    }

    pub fn state(mut self, name: &str, kind: LinearStateKind) -> Self {
        self.issue.state.name = name.to_owned();
        self.issue.state.kind = kind;
        self
    }

    pub fn priority(mut self, priority: LinearPriority) -> Self {
        self.issue.priority = priority;
        self
    }

    pub fn cycle(mut self, number: u64, active: bool) -> Self {
        self.issue.cycle = Some(LinearCycle {
            id: format!("cycle-{number}"),
            number,
            name: None,
            active,
        });
        self
    }

    pub fn updated(mut self, at: &str) -> Self {
        self.issue.updated_at = ts(at);
        self
    }

    pub fn build(self) -> LinearIssue {
        self.issue
    }
}
