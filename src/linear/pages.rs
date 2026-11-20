//! Reading one paginated Linear list: the paging loop every Linear list
//! shares (the assigned issues, the team states, the start teams' issues).
//! A [`Pager`] (the pager shared with GitHub, `crate::paging`) keeps the
//! list's state across calls, so one list can be read in several parts
//! against one page bound (the start teams' issues, by priority). The bound
//! is the caller's: a list may stop short of [`super::MAX_LINEAR_PAGES`] (one
//! page, to see whether there are more), and its reason may say which end
//! was cut, so [`read_list`] returns [`End::Cut`] rather than noting it.

use super::{
    Connection,
    session::{Answer, Pager, Session},
};
use crate::domain::{Incompleteness, SourceState};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// How reading a list ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    /// The last page was read.
    Done,
    /// A page was lost, or Linear gave no cursor for the next one; the
    /// pager already says so.
    Stopped,
    /// `max_pages` pages were read and Linear has more. The pager does not
    /// say so yet: the caller knows best which end of the list was cut (see
    /// [`page_bound`]).
    Cut,
}

/// One query of a paginated list: its operation name and text, its
/// variables for a cursor, and where the connection is in its data.
pub(super) struct List<'q, V, C> {
    pub operation: &'q str,
    pub query: &'q str,
    pub variables: V,
    pub connection: C,
}

/// Reads the pages of `list` from its first, handing each node to `each`
/// with whether its page came without errors (#35: errors next to data leave
/// that page's attachments incomplete), until the last page, a lost page, or
/// `pager` having read `max_pages` in all. Each page's state goes into `pager`.
pub(super) fn read_list<T, V, C>(
    session: &Session<'_>,
    pager: &mut Pager,
    max_pages: u32,
    list: &List<'_, V, C>,
    mut each: impl FnMut(Value, bool),
) -> End
where
    T: DeserializeOwned,
    V: Fn(Option<&str>) -> Value,
    C: Fn(T) -> Connection,
{
    pager.cursor = None;
    loop {
        let variables = (list.variables)(pager.cursor.as_deref());
        let page = match session.run::<T>(list.operation, list.query, variables) {
            Answer::Lost(state) => {
                pager.page_lost(state);
                return End::Stopped;
            }
            Answer::Data { data, state } => {
                let complete = state.is_complete();
                pager.page_read(state);
                ((list.connection)(data), complete)
            }
        };
        let (page, page_complete) = page;
        for node in page.nodes {
            each(node, page_complete);
        }
        let more = page.page_info.has_next_page;
        // The shared pager moves to the next page; more pages without a
        // cursor it notes as partial.
        if !pager.advance(more, page.page_info.end_cursor) {
            return if more { End::Stopped } else { End::Done };
        }
        if pager.pages_read() >= max_pages {
            return End::Cut;
        }
    }
}

/// The partial state of a list cut at den's page bound; `which` says what
/// was and wasn't read, when the caller knows.
pub(super) fn page_bound(pager: &Pager, which: Option<&str>) -> SourceState {
    let bound = Pager::bound_reason();
    SourceState::Partial(Incompleteness::PaginationStopped {
        pages_read: pager.pages_read(),
        reason: match which {
            Some(which) => format!("{bound}; {which}"),
            None => bound,
        },
    })
}

/// Notes the nodes den could not read (`what` names them), if any.
pub(super) fn note_unreadable(pager: &mut Pager, unreadable: &[String], what: &str) {
    if let Some(first) = unreadable.first() {
        pager.note(SourceState::Partial(Incompleteness::FollowUpTruncated {
            what: format!(
                "{} {what} den could not read, first: {first}",
                unreadable.len()
            ),
        }));
    }
}
