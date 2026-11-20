//! Waiting On: resolving the person who owes a step to me / someone else /
//! nobody (+ unattributed). Shared by every kind's rules.

use crate::domain::{LinearViewer, TeamRef, Viewer, WaitingOn, facts::LinearUser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    pub waiting_on: WaitingOn,
    pub reason: String,
    pub unattributed: bool,
}

impl Attribution {
    /// The step is owed by one person, described by `role` ("the PR author").
    /// An unknown person is Waiting On nobody and unattributed.
    pub fn person(viewer: &Viewer, login: Option<&str>, role: &str) -> Self {
        match login {
            Some(login) if viewer.is(login) => Self {
                waiting_on: WaitingOn::Me,
                reason: format!("you are {role} ({login})"),
                unattributed: false,
            },
            Some(login) => Self {
                waiting_on: WaitingOn::SomeoneElse,
                reason: format!("{role} is {login}"),
                unattributed: false,
            },
            None => Self {
                waiting_on: WaitingOn::Nobody,
                reason: format!("den could not establish {role}"),
                unattributed: true,
            },
        }
    }

    /// The step is owed by any of several people or teams; me if I am any of them.
    ///
    /// `unread` names what den could not read that might make it me (requests
    /// it did not see, team memberships it does not know). A match to me still
    /// counts; without one, it is unattributed rather than someone else's.
    pub fn any_of(
        viewer: &Viewer,
        logins: &[String],
        teams: &[TeamRef],
        role: &str,
        unread: &[String],
    ) -> Self {
        let me_directly = logins.iter().any(|l| viewer.is(l));
        let my_team = teams.iter().find(|t| viewer.belongs_to(t));
        if me_directly {
            return Self {
                waiting_on: WaitingOn::Me,
                reason: format!("you are one of {role}"),
                unattributed: false,
            };
        }
        if let Some(team) = my_team {
            return Self {
                waiting_on: WaitingOn::Me,
                reason: format!("your team {}/{} is one of {role}", team.org, team.slug),
                unattributed: false,
            };
        }
        if !unread.is_empty() {
            return Self {
                waiting_on: WaitingOn::Nobody,
                reason: format!(
                    "den could not establish whether you are one of {role}: {}",
                    unread.join("; ")
                ),
                unattributed: true,
            };
        }
        if logins.is_empty() && teams.is_empty() {
            return Self::nobody(format!("there are no {role}"));
        }
        Self {
            waiting_on: WaitingOn::SomeoneElse,
            reason: format!("you are not one of {role}"),
            unattributed: false,
        }
    }

    /// The step is owed by one Linear user, described by `role` ("the Linear
    /// assignee"). Me only when that user is the Linear viewer of the issue's
    /// workspace, compared by Linear's user ID: never by name, never against a
    /// GitHub login. Without a Linear viewer den can't tell, so it is Waiting
    /// On nobody and unattributed.
    pub fn linear_person(
        viewer: Option<&LinearViewer>,
        workspace: &str,
        user: Option<&LinearUser>,
        role: &str,
    ) -> Self {
        let Some(user) = user else {
            return Self::nobody(format!("there is no {role}"));
        };
        match viewer {
            Some(viewer) if viewer.is(&user.id) => Self {
                waiting_on: WaitingOn::Me,
                reason: format!("you are {role} ({}, your Linear account)", user.name),
                unattributed: false,
            },
            Some(_) => Self {
                waiting_on: WaitingOn::SomeoneElse,
                reason: format!("{role} is {}", user.name),
                unattributed: false,
            },
            None => Self {
                waiting_on: WaitingOn::Nobody,
                reason: format!(
                    "{role} is {}, but den could not read who you are in the Linear workspace {workspace}",
                    user.name
                ),
                unattributed: true,
            },
        }
    }

    /// Nobody owes anything (no Next Step, or a step nobody in particular owes).
    pub fn nobody(reason: impl Into<String>) -> Self {
        Self {
            waiting_on: WaitingOn::Nobody,
            reason: reason.into(),
            unattributed: false,
        }
    }
}
