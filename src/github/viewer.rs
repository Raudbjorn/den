//! The viewer: login, then team memberships (for team review requests).

use super::session::{Fetched, Session};
use crate::{
    domain::{CoverageEntry, Incompleteness, Source, SourceState, TeamRef, Viewer},
    ports::SurveyFailure,
};
use serde::Deserialize;
use serde_json::json;

const VIEWER_QUERY: &str = include_str!("queries/viewer.graphql");
const TEAMS_QUERY: &str = include_str!("queries/viewer_teams.graphql");

#[derive(Deserialize)]
struct LoginData {
    viewer: Login,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

#[derive(Deserialize)]
struct TeamsData {
    viewer: TeamsViewer,
}

#[derive(Deserialize)]
struct TeamsViewer {
    organizations: super::session::Connection<Org>,
}

#[derive(Deserialize)]
struct Org {
    login: String,
    teams: super::session::Connection<Team>,
}

#[derive(Deserialize)]
struct Team {
    slug: String,
}

/// Fetches the viewer. Without a login, Waiting On can't be decided, so a lost
/// login stops the survey; lost teams only make the viewer source partial.
pub fn fetch(session: &Session<'_>) -> Result<(Viewer, CoverageEntry), SurveyFailure> {
    let login = match session.run::<LoginData>(VIEWER_QUERY, json!({}))? {
        Fetched::Data { data, .. } => data.viewer.login,
        Fetched::Lost(state) => {
            return Err(SurveyFailure::ViewerUnavailable {
                detail: state.describe(),
            });
        }
    };
    let (teams, state) = match session.run::<TeamsData>(TEAMS_QUERY, json!({ "login": login }))? {
        Fetched::Data { data, state } => {
            let orgs = data.viewer.organizations;
            let mut truncated = orgs.page_info.has_next_page;
            let mut teams = Vec::new();
            for org in orgs.nodes.into_iter().flatten() {
                truncated |= org.teams.page_info.has_next_page;
                teams.extend(org.teams.nodes.into_iter().flatten().map(|t| TeamRef {
                    org: org.login.clone(),
                    slug: t.slug,
                }));
            }
            let state = if truncated {
                state.worst(SourceState::Partial(Incompleteness::PaginationStopped {
                    pages_read: 1,
                    reason: "more than 100 organizations or teams; den read the first 100"
                        .to_owned(),
                }))
            } else {
                state
            };
            (teams, state)
        }
        Fetched::Lost(state) => (
            Vec::new(),
            SourceState::Partial(Incompleteness::ErrorsAlongsideData {
                errors: vec![format!("team memberships unknown: {}", state.describe())],
            }),
        ),
    };
    Ok((
        Viewer { login, teams },
        CoverageEntry {
            source: Source::Viewer,
            state,
        },
    ))
}
