//! The sidebar: every Repo of the watched Owners, grouped by Owner, with its
//! count of Work Items that have a Next Step, and whether it is a fork or
//! archived, and, for a fork that targets its Upstream, which Upstream (#13).
//! Each row is a Scope the user can switch to.

use super::{State, coverage};
use crate::{
    app::{
        Focus,
        view_model::{Sidebar, SidebarEntry, SidebarRow},
    },
    ordering::{Scope, in_scope, same_repo},
};

const LABEL: &str = "R E P O S";
const ALL_OWNERS: &str = "All Owners";

pub fn build(state: &State<'_>) -> Sidebar {
    let rows = state
        .sidebar
        .iter()
        .map(|scope| row(state, scope))
        .collect();
    Sidebar {
        label: LABEL.to_owned(),
        rows,
        focused: state.scope.focus == Focus::Sidebar,
        cursor: state.scope.cursor_index(&state.sidebar),
    }
}

fn row(state: &State<'_>, scope: &Scope) -> SidebarRow {
    let count = in_scope(state.all_decisions, scope)
        .filter(|d| d.next_step.is_some())
        .count();
    let (entry, label, repo) = match scope {
        Scope::AllOwners => (SidebarEntry::AllOwners, ALL_OWNERS.to_owned(), None),
        Scope::Owner(owner) => (SidebarEntry::Owner, owner.clone(), None),
        Scope::Repo(id) => (
            SidebarEntry::Repo,
            id.name.clone(),
            state
                .observation
                .and_then(|o| o.repos.iter().find(|r| same_repo(&r.id, id))),
        ),
    };
    SidebarRow {
        scope: scope.clone(),
        entry,
        label,
        count,
        fork: repo.is_some_and(|r| r.is_fork),
        upstream: repo
            .filter(|r| r.is_fork && state.upstreams.contains(&r.id))
            .map(|r| {
                r.upstream()
                    .map_or_else(|| "(not visible)".to_owned(), |u| u.id.to_string())
            }),
        archived: repo.is_some_and(|r| r.is_archived),
        current: state.scope.scope.same(scope),
        linear_workflow: matches!(scope, Scope::Owner(owner) if super::has_linear_workflow(state, owner)),
        // No Observation yet is not "incomplete coverage": nothing was read.
        incomplete: state
            .observation
            .is_some_and(|observation| !coverage::scope_complete(observation, scope)),
    }
}
