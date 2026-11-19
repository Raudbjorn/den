//! The header's Scope: a sentence saying what the list covers, and the
//! segmented Scope control.

use super::{State, coverage};
use crate::{
    app::view_model::{EmptyState, ScopeSegment},
    ordering::{Scope, same_repo},
};

const ALL_OWNERS: &str = "All Owners";
const NO_OWNERS: &str = "No Owners configured";

/// `(sentence, segments)`, e.g. ("Repo · octo/app · fork of up/app", [All
/// Owners, octo, app*]).
pub fn describe(state: &State<'_>) -> (String, Vec<ScopeSegment>) {
    if state.owners.is_empty() {
        return (NO_OWNERS.to_owned(), Vec::new());
    }
    let current = &state.scope.scope;
    let sentence = match current {
        Scope::AllOwners => format!("{ALL_OWNERS} · {}", state.owners.join(", ")),
        Scope::Owner(owner) => format!("Owner · {owner}"),
        Scope::Repo(id) => {
            let mut sentence = format!("Repo · {id}");
            let repo = state
                .observation
                .and_then(|o| o.repos.iter().find(|r| same_repo(&r.id, id)));
            if let Some(repo) = repo {
                match (&repo.parent, repo.is_fork) {
                    (Some(parent), true) => sentence.push_str(&format!(" · fork of {parent}")),
                    (None, true) => sentence.push_str(" · fork"),
                    _ => {}
                }
                if repo.is_archived {
                    sentence.push_str(" · archived");
                }
            }
            sentence
        }
    };
    let mut segments = vec![segment(ALL_OWNERS, current == &Scope::AllOwners)];
    segments.extend(
        state
            .owners
            .iter()
            .map(|owner| segment(owner, current == &Scope::Owner(owner.clone()))),
    );
    if let Scope::Repo(id) = current {
        segments.push(segment(&id.name, true));
    }
    (sentence, segments)
}

fn segment(label: &str, active: bool) -> ScopeSegment {
    ScopeSegment {
        label: label.to_owned(),
        active,
    }
}

/// The empty state of a narrowed Scope that has nothing in it while other
/// Scopes do: it must not read as "nothing needs you". `None` when the Scope is
/// All Owners or there is no Observation (the general empty states apply).
pub fn empty_in_scope(state: &State<'_>) -> Option<EmptyState> {
    let observation = state.observation?;
    let current = &state.scope.scope;
    if current == &Scope::AllOwners {
        return None;
    }
    let elsewhere = state.all_decisions.len();
    let title = if coverage::scope_complete(observation, current) {
        format!("Nothing in {current}.")
    } else {
        format!("No Work Items found in {current}, but the picture is incomplete.")
    };
    let detail = match elsewhere {
        0 => "Nothing in any other Scope either; ← → or Tab to switch Scope.".to_owned(),
        1 => "1 Work Item is in another Scope; ← → or Tab to switch Scope.".to_owned(),
        n => format!("{n} Work Items are in other Scopes; ← → or Tab to switch Scope."),
    };
    Some(EmptyState { title, detail })
}
