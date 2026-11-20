//! Linear Issue facts: what the Linear source observed about one Linear Issue
//! of an Owner with a Linear Workflow (spec 0002, "Facts: Linear Issue").
//! Nothing decided. Attachments (the PRs Linear recorded) arrive with Landing
//! Groups (#35).

use crate::domain::ids::{LinearIssueId, WorkItemId};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearIssue {
    /// The Owner whose Linear Workflow surveyed it.
    pub owner: String,
    /// The identity: workspace + Linear's permanent ID.
    pub id: LinearIssueId,
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
