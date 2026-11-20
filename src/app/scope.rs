//! Scope switching and the sidebar's cursor (pure; part of seam 2).
//!
//! The sidebar is a list of Scopes: All Owners, then each watched Owner (config
//! order) followed by its Repos. `Tab` moves focus between the list and the
//! sidebar; in the sidebar `↑`/`↓` move the cursor and `Enter` scopes to the row
//! under it. `←`/`→` step through the segmented control (All Owners, each
//! Owner) from either focus.

use super::event::Key;
use crate::{
    decide::Decision,
    domain::{Observation, RepoId, ids::same_login},
    ordering::{Scope, same_repo},
};

/// Which pane the arrow keys move in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    List,
    Sidebar,
}

/// What a key did to the Scope state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The key is not a Scope key here; the App handles it.
    Unhandled,
    /// Focus or the cursor moved; the Scope is unchanged.
    Moved,
    /// The Scope changed (to the value now in `ScopeState::scope`).
    Changed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeState {
    pub scope: Scope,
    pub focus: Focus,
    /// The sidebar row under the cursor, by identity (survives a refresh).
    pub cursor: Scope,
}

impl ScopeState {
    /// Handles a Scope key. `entries` are the sidebar's rows, in order.
    pub fn key(&mut self, key: Key, owners: &[String], entries: &[Scope]) -> Outcome {
        match (key, self.focus) {
            // Quit and refresh work whichever pane has focus.
            (Key::Char('q' | 'r') | Key::Interrupt, _) => Outcome::Unhandled,
            (Key::Tab, Focus::List) => {
                self.focus = Focus::Sidebar;
                if !entries.iter().any(|e| e.same(&self.cursor)) {
                    self.cursor = self.scope.clone();
                }
                Outcome::Moved
            }
            (Key::Tab | Key::Esc, Focus::Sidebar) => {
                self.focus = Focus::List;
                Outcome::Moved
            }
            (Key::Left, _) => self.set(step_segment(&self.scope, owners, Direction::Back)),
            (Key::Right, _) => self.set(step_segment(&self.scope, owners, Direction::Forward)),
            (Key::Up, Focus::Sidebar) => {
                self.cursor = step_cursor(&self.shown_cursor(entries), entries, Direction::Back);
                Outcome::Moved
            }
            (Key::Down, Focus::Sidebar) => {
                self.cursor = step_cursor(&self.shown_cursor(entries), entries, Direction::Forward);
                Outcome::Moved
            }
            // The row the user sees under the cursor, not a cursor a refresh
            // left pointing at a Repo that no longer has a row.
            (Key::Enter, Focus::Sidebar) => {
                self.focus = Focus::List;
                self.set(self.shown_cursor(entries))
            }
            // `u` switches the fork under the cursor between targeting itself
            // and its Upstream (#13); the App handles it.
            (Key::Char('u'), Focus::Sidebar) => Outcome::Unhandled,
            // While the sidebar has focus, other keys do nothing.
            (_, Focus::Sidebar) => Outcome::Moved,
            (_, Focus::List) => Outcome::Unhandled,
        }
    }

    fn set(&mut self, scope: Scope) -> Outcome {
        self.cursor = scope.clone();
        if scope.same(&self.scope) {
            return Outcome::Moved;
        }
        self.scope = scope;
        Outcome::Changed
    }

    /// The cursor's row index in `entries`, falling back to the Scope's row,
    /// then to the first row (All Owners) when neither has a row any more.
    pub fn cursor_index(&self, entries: &[Scope]) -> Option<usize> {
        entries
            .iter()
            .position(|e| e.same(&self.cursor))
            .or_else(|| entries.iter().position(|e| e.same(&self.scope)))
            .or_else(|| (!entries.is_empty()).then_some(0))
    }

    /// The Scope of the row drawn under the cursor (see [`Self::cursor_index`]).
    /// With no rows at all the cursor stays where it was.
    fn shown_cursor(&self, entries: &[Scope]) -> Scope {
        self.cursor_index(entries)
            .and_then(|i| entries.get(i))
            .unwrap_or(&self.cursor)
            .clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Back,
    Forward,
}

/// The segmented control: All Owners, then each Owner, wrapping at both ends.
/// From a Repo, back widens to its Owner and forward goes to the next Owner.
fn step_segment(scope: &Scope, owners: &[String], direction: Direction) -> Scope {
    let segments: Vec<Scope> = std::iter::once(Scope::AllOwners)
        .chain(owners.iter().map(|o| Scope::Owner(o.clone())))
        .collect();
    let owner_segment = |owner: &str| {
        owners
            .iter()
            .position(|o| same_login(o, owner))
            .map(|i| i + 1)
    };
    let current = match scope {
        Scope::AllOwners => Some(0),
        Scope::Owner(owner) => owner_segment(owner),
        Scope::Repo(repo) => match (direction, owner_segment(&repo.owner)) {
            (Direction::Back, Some(i)) => return segments.get(i).cloned().unwrap_or_default(),
            (_, found) => found,
        },
    };
    let count = segments.len();
    let next = match (current, direction) {
        (None, _) => 0,
        (Some(i), Direction::Forward) => (i + 1) % count,
        (Some(i), Direction::Back) => (i + count - 1) % count,
    };
    segments.get(next).cloned().unwrap_or_default()
}

/// Moves the cursor one row, stopping at the ends.
fn step_cursor(cursor: &Scope, entries: &[Scope], direction: Direction) -> Scope {
    let Some(last) = entries.len().checked_sub(1) else {
        return cursor.clone();
    };
    let index = match (entries.iter().position(|e| e.same(cursor)), direction) {
        (None, _) => 0,
        (Some(i), Direction::Forward) => (i + 1).min(last),
        (Some(i), Direction::Back) => i.saturating_sub(1),
    };
    entries
        .get(index)
        .cloned()
        .unwrap_or_else(|| cursor.clone())
}

/// The sidebar's rows, as Scopes: All Owners, then for each Owner in config
/// order the Owner and its Repos (by name, case-insensitively). A Repo appears
/// if the Observation lists it or any Decision belongs to it, so a Work Item is
/// never orphaned when the Repo list was read incompletely.
pub fn entries(
    owners: &[String],
    observation: Option<&Observation>,
    decisions: &[Decision],
) -> Vec<Scope> {
    let mut repos: Vec<&RepoId> = Vec::new();
    let listed = observation
        .into_iter()
        .flat_map(|o| o.repos.iter().map(|r| &r.id));
    for repo in listed.chain(decisions.iter().filter_map(|d| d.item.id.repo())) {
        if !repos.iter().any(|known| same_repo(known, repo)) {
            repos.push(repo);
        }
    }
    repos.sort_by_key(|r| r.name.to_lowercase());
    let mut rows = vec![Scope::AllOwners];
    for owner in owners {
        rows.push(Scope::Owner(owner.clone()));
        rows.extend(
            repos
                .iter()
                .filter(|r| same_login(&r.owner, owner))
                .map(|r| Scope::Repo((*r).clone())),
        );
    }
    rows
}
