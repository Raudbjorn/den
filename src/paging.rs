//! Walking one paginated GraphQL list, shared by the GitHub and Linear
//! sources: pages read, the page bound, and the worst state so far. Reaching
//! the bound, a page lost after earlier pages, or "more pages" without a
//! cursor is partial coverage, never an invented count.

use crate::domain::{Incompleteness, SourceState};
use std::marker::PhantomData;

/// The service a list is read from: its page bound and how its reasons read.
pub trait PagedService {
    /// Upper bound on pages read per list, so a runaway pagination can't run
    /// forever. Reaching it is recorded as partial coverage.
    const MAX_PAGES: u32;
    /// The service's name, in reasons ("GitHub reported more pages …").
    const NAME: &'static str;
    /// What one of its lists is called in the page-bound reason.
    const LIST: &'static str;
}

/// Tracks one paginated list of `S`.
pub struct Pager<S: PagedService> {
    pub cursor: Option<String>,
    pages_read: u32,
    state: SourceState,
    service: PhantomData<S>,
}

impl<S: PagedService> Default for Pager<S> {
    /// Before the first page: no cursor, nothing read, nothing wrong yet.
    fn default() -> Self {
        Self::new(None)
    }
}

impl<S: PagedService> Pager<S> {
    /// Starts at `cursor` (`None`: the first page).
    pub fn new(cursor: Option<String>) -> Self {
        Self {
            cursor,
            pages_read: 0,
            state: SourceState::Complete,
            service: PhantomData,
        }
    }

    /// False once [`PagedService::MAX_PAGES`] pages were read; the bound is
    /// recorded as partial.
    pub fn may_continue(&mut self) -> bool {
        if self.pages_read < S::MAX_PAGES {
            return true;
        }
        self.note(SourceState::Partial(Incompleteness::PaginationStopped {
            pages_read: self.pages_read,
            reason: Self::bound_reason(),
        }));
        false
    }

    /// Why a list stopped at the page bound, as its partial reason reads.
    pub fn bound_reason() -> String {
        format!("den reads at most {} pages per {}", S::MAX_PAGES, S::LIST)
    }

    pub fn pages_read(&self) -> u32 {
        self.pages_read
    }

    /// A page arrived with this state.
    pub fn page_read(&mut self, state: SourceState) {
        self.pages_read += 1;
        self.note(state);
    }

    /// A page was lost. After earlier pages, that makes the list partial; a
    /// rate limit stays a rate limit, with its reset time.
    pub fn page_lost(&mut self, state: SourceState) {
        let state = match (self.pages_read, state) {
            (0, state) => state,
            (_, SourceState::Partial(why)) if why.rate_limit_reset().is_some() => {
                SourceState::Partial(why)
            }
            (pages_read, state) => SourceState::Partial(Incompleteness::PaginationStopped {
                pages_read,
                reason: state.describe(),
            }),
        };
        self.note(state);
    }

    pub fn note(&mut self, state: SourceState) {
        let previous = std::mem::replace(&mut self.state, SourceState::Complete);
        self.state = previous.worst(state);
    }

    /// Moves to the next page, or returns false when there is none. More
    /// pages without a cursor stop the list as partial.
    pub fn advance(&mut self, has_next_page: bool, end_cursor: Option<String>) -> bool {
        match (has_next_page, end_cursor) {
            (true, Some(cursor)) => {
                self.cursor = Some(cursor);
                true
            }
            (true, None) => {
                self.note(SourceState::Partial(Incompleteness::PaginationStopped {
                    pages_read: self.pages_read,
                    reason: format!("{} reported more pages but no cursor", S::NAME),
                }));
                false
            }
            (false, _) => false,
        }
    }

    /// The list's state: the worst of its pages.
    pub fn into_state(self) -> SourceState {
        self.state
    }
}
