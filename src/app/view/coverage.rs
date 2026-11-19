//! Coverage in words, shared by the header, the list's empty state, the detail
//! pane and the sidebar: what was not read completely, the age of any data kept
//! for it, and when a rate limit resets. Never a count of unseen items.

use super::{State, format};
use crate::{
    app::view_model::RepoCoverage,
    cache::{self, source_owner},
    domain::{CoverageEntry, Incompleteness, Observation, RepoId, Source, SourceState},
    ordering::{Scope, same_repo},
};
use jiff::Timestamp;

/// "showing data from 10:00 UTC, 2 h ago" when den kept earlier data for
/// `source` because the latest survey could not read it.
pub fn carried_note(state: &State<'_>, source: &Source) -> Option<String> {
    let since = carried_since(state, source)?;
    Some(format!(
        "showing data from {}, {}",
        format::clock_time(since),
        format::ago(state.now, since)
    ))
}

/// When the data shown for an item was observed: the oldest age of data kept
/// for any of its sources, else when the Observation was made.
pub fn item_observed_at(
    state: &State<'_>,
    sources: &[CoverageEntry],
    fresh: Timestamp,
) -> Timestamp {
    sources
        .iter()
        .filter_map(|entry| carried_since(state, &entry.source))
        .min()
        .map_or(fresh, |kept| kept.min(fresh))
}

fn carried_since(state: &State<'_>, source: &Source) -> Option<Timestamp> {
    let own = state
        .carried
        .iter()
        .find(|c| &c.source == source)
        .map(|c| c.observed_at);
    own.or_else(|| match source {
        Source::Repo { repo, kind } => carried_since(
            state,
            &Source::Owner {
                owner: repo.owner.clone(),
                kind: *kind,
            },
        ),
        Source::Owner { .. } | Source::Viewer => None,
    })
}

/// "Pull Requests of alpha: failed: timeout; showing data from 10:00 UTC, 2 h ago".
pub fn entry_line(state: &State<'_>, entry: &CoverageEntry) -> String {
    let line = format!("{}: {}", entry.source.label(), entry.state.describe());
    match carried_note(state, &entry.source) {
        Some(note) => format!("{line}; {note}"),
        None => line,
    }
}

/// The latest rate-limit reset any source reported, as a note for the status
/// line: "rate limited until 13:00 UTC". `None` when nothing was rate limited.
pub fn rate_limit_note(observation: &Observation) -> Option<String> {
    let mut limited = false;
    let mut latest: Option<Timestamp> = None;
    for entry in observation.coverage.entries() {
        if let SourceState::Partial(Incompleteness::RateLimited { resets_at }) = &entry.state {
            limited = true;
            latest = latest.max(*resets_at);
        }
    }
    limited.then(|| match latest {
        Some(at) => format!("rate limited until {}", format::clock_time(at)),
        None => "rate limited; GitHub did not say when it resets".to_owned(),
    })
}

/// True when everything behind `scope` was read completely: the viewer (for
/// Waiting On) and every source of the Owner or Repo the Scope covers. All
/// Owners is the whole survey, so it is [`cache::survey_complete`], the only gate
/// for "Nothing needs you".
pub fn scope_complete(observation: &Observation, scope: &Scope) -> bool {
    let viewer_complete = observation
        .coverage
        .get(&Source::Viewer)
        .is_some_and(SourceState::is_complete);
    match scope {
        Scope::AllOwners => cache::survey_complete(observation),
        Scope::Owner(owner) => {
            let entries: Vec<&CoverageEntry> = observation
                .coverage
                .entries()
                .iter()
                .filter(|e| source_owner(&e.source).is_some_and(|o| o.eq_ignore_ascii_case(owner)))
                .collect();
            viewer_complete && !entries.is_empty() && entries.iter().all(|e| e.state.is_complete())
        }
        Scope::Repo(repo) => {
            let applicable = applicable_to_repo(observation, repo);
            viewer_complete
                && !applicable.is_empty()
                && applicable.iter().all(|e| e.state.is_complete())
        }
    }
}

/// The sources behind one Repo's Work Items: its Owner's and its own.
fn applicable_to_repo<'o>(observation: &'o Observation, repo: &RepoId) -> Vec<&'o CoverageEntry> {
    observation
        .coverage
        .entries()
        .iter()
        .filter(|e| match &e.source {
            Source::Owner { owner, .. } => owner.eq_ignore_ascii_case(&repo.owner),
            Source::Repo { repo: r, .. } => same_repo(r, repo),
            Source::Viewer => false,
        })
        .collect()
}

/// How completely one Repo was read: its Owner's sources and its own.
pub fn for_repo(state: &State<'_>, observation: &Observation, repo: &RepoId) -> RepoCoverage {
    let applicable = applicable_to_repo(observation, repo);
    let incomplete: Vec<&CoverageEntry> = applicable
        .iter()
        .copied()
        .filter(|e| !e.state.is_complete())
        .collect();
    let complete = !applicable.is_empty() && incomplete.is_empty();
    let problems: Vec<String> = incomplete
        .iter()
        .map(|e| {
            let line = format!("{}: {}", kind_label(&e.source), e.state.describe());
            match carried_note(state, &e.source) {
                Some(note) => format!("{line}; {note}"),
                None => line,
            }
        })
        .collect();
    let summary = if complete {
        "complete".to_owned()
    } else if applicable.is_empty() {
        "not read".to_owned()
    } else {
        format!("incomplete: {}", short_summary(&incomplete))
    };
    RepoCoverage {
        repo: repo.to_string(),
        complete,
        summary,
        problems,
    }
}

/// Coverage of every Repo in the Observation, in survey order.
pub fn by_repo(state: &State<'_>) -> Vec<RepoCoverage> {
    state.observation.map_or_else(Vec::new, |observation| {
        observation
            .repos
            .iter()
            .map(|repo| for_repo(state, observation, &repo.id))
            .collect()
    })
}

fn kind_label(source: &Source) -> String {
    match source {
        Source::Owner { kind, .. } | Source::Repo { kind, .. } => kind.label().to_owned(),
        Source::Viewer => source.label(),
    }
}

/// "Pull Requests failed; not fetched yet: Issues, Runs".
fn short_summary(incomplete: &[&CoverageEntry]) -> String {
    let mut troubled: Vec<String> = Vec::new();
    let mut not_fetched: Vec<String> = Vec::new();
    for entry in incomplete {
        let label = kind_label(&entry.source);
        let target = match &entry.state {
            SourceState::NotYetFetched => &mut not_fetched,
            SourceState::Failed { .. } => {
                push_unique(&mut troubled, format!("{label} failed"));
                continue;
            }
            SourceState::Partial(_) => {
                push_unique(&mut troubled, format!("{label} partial"));
                continue;
            }
            SourceState::Complete => continue,
        };
        push_unique(target, label);
    }
    let mut parts = troubled;
    if !not_fetched.is_empty() {
        parts.push(format!("not fetched yet: {}", not_fetched.join(", ")));
    }
    parts.join("; ")
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}
