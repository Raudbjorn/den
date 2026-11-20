//! A fork's Upstream as its target (#13, ADR-0003). `u` in the sidebar, on a
//! fork's row, switches that fork's target between itself and its Upstream; the
//! choice is saved in den's state directory (`Effect::StoreUpstreams`) and read
//! back at the next start (`App::with_upstreams`). `u` does nothing on any other
//! row. The launch sheet's `u` (one Job only) is in `launch.rs`.
//!
//! Switching changes what the fork's Branches should be compared against. den
//! does not survey again by itself (a survey of every Owner is costly, and the
//! switch may be flipped straight back): the Branches are decided again at
//! once, and those counted against the other branch get no Next Step, with an
//! Explanation saying to refresh (`r`). The next survey compares them against
//! the new target.

use super::{App, Effect, LogLevel};
use crate::{
    domain::{Repo, UpstreamTargets},
    ordering::{Scope, same_repo},
    upstreams::UpstreamStoreError,
};

impl App {
    /// Sets the Upstream choices read at startup, or why they couldn't be read.
    /// Unreadable choices count as none (every fork is its own target) and `u`
    /// refuses to change them, so a file den can't read is never overwritten.
    pub fn with_upstreams(
        mut self,
        upstreams: Result<UpstreamTargets, UpstreamStoreError>,
    ) -> Self {
        match upstreams {
            Ok(upstreams) => {
                self.upstreams = upstreams;
                self.upstreams_unreadable = None;
            }
            Err(error) => {
                self.upstreams = UpstreamTargets::default();
                self.upstreams_unreadable = Some(error.to_string());
            }
        }
        self
    }

    /// The fork under the sidebar cursor, if the cursor is on a fork's row.
    fn fork_under_cursor(&self) -> Option<&Repo> {
        let Scope::Repo(id) = &self.scope.cursor else {
            return None;
        };
        self.shown
            .as_ref()?
            .observation
            .repos
            .iter()
            .find(|r| same_repo(&r.id, id))
            .filter(|r| r.is_fork)
    }

    /// `u` in the sidebar.
    pub(super) fn toggle_upstream(&mut self) -> Vec<Effect> {
        let Some(fork) = self.fork_under_cursor() else {
            // Not a fork (or not a Repo row): nothing to switch.
            return Vec::new();
        };
        let fork_id = fork.id.clone();
        let upstream = fork.upstream().map(|u| u.id.clone());
        if self.upstreams_unreadable.is_some() {
            return vec![Effect::log(
                LogLevel::Warn,
                format!(
                    "u on {fork_id} ignored: den can't read its Upstream choices, so it won't overwrite them"
                ),
            )];
        }
        // Switching back to the fork itself never needs the Upstream; switching
        // to an Upstream den can't see would make a meaningless target.
        if !self.upstreams.contains(&fork_id) && upstream.is_none() {
            self.upstream_problem = Some(format!(
                "{fork_id}'s Upstream is not visible to den, so it can't be the target"
            ));
            return vec![Effect::log(
                LogLevel::Warn,
                format!("u on {fork_id} ignored: den can't see its Upstream"),
            )];
        }
        let now_upstream = self.upstreams.toggle(&fork_id);
        self.upstream_problem = None;
        self.redecide();
        let message = match (now_upstream, upstream) {
            (true, Some(upstream)) => format!("{fork_id} now targets its Upstream {upstream}"),
            (true, None) => format!("{fork_id} now targets its Upstream"),
            (false, _) => format!("{fork_id} now targets itself, not its Upstream"),
        };
        vec![
            Effect::log(LogLevel::Info, message),
            Effect::StoreUpstreams {
                upstreams: self.upstreams.clone(),
            },
        ]
    }

    /// What the header says about the Upstream choices, when something is wrong.
    pub(super) fn upstream_note(&self) -> Option<String> {
        match (&self.upstreams_unreadable, &self.upstream_problem) {
            (Some(error), _) => Some(format!(
                "Upstream choices unreadable, so every fork targets itself: {error}"
            )),
            (None, problem) => problem.clone(),
        }
    }

    pub(super) fn upstreams_stored(
        &mut self,
        outcome: Result<(), UpstreamStoreError>,
    ) -> Vec<Effect> {
        match outcome {
            Ok(()) => Vec::new(),
            Err(error) => {
                self.upstream_problem = Some(format!(
                    "Upstream choice not saved, so it lasts until den quits: {error}"
                ));
                vec![Effect::log(
                    LogLevel::Error,
                    format!("could not save the Upstream choices: {error}"),
                )]
            }
        }
    }
}
