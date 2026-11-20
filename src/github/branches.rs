//! Branches of an Owner's Repos. Not fetched yet: ticket #6 adds the query
//! (in `queries/`) and the parsing here. Until then the source is recorded as
//! not yet fetched, so the view never presents missing Branches as none.

use super::session::{KindFetch, Session};
use crate::{
    domain::{CoverageEntry, Repo, Source, SourceKind, SourceState, facts::Branch},
    ports::SurveyFailure,
};

pub fn fetch(
    _session: &Session<'_>,
    owner: &str,
    _repos: &[Repo],
) -> Result<KindFetch<Branch>, SurveyFailure> {
    Ok(KindFetch {
        items: Vec::new(),
        coverage: vec![CoverageEntry {
            source: Source::Owner {
                owner: owner.to_owned(),
                kind: SourceKind::Branches,
            },
            state: SourceState::NotYetFetched,
        }],
    })
}
