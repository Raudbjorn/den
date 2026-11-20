//! The reviewers a Request review Job asks (#39): the sheet's editable list,
//! spelled and parsed. Each is a GitHub login (`alice`) or a team
//! (`acme/backend`), comma- or space-separated, as `gh` takes them. The one
//! reviewer grammar: the config's `reviewers` parse and compare through
//! [`parse`] and [`folded`] too.

use crate::{
    config::is_valid_team_slug,
    domain::{TeamRef, facts::Reviewer, ids::is_valid_login},
};

/// A reviewer as the sheet and the prompt spell it: `alice`, `acme/backend`.
pub fn spell(reviewer: &Reviewer) -> String {
    match reviewer {
        Reviewer::User(login) => login.clone(),
        Reviewer::Team(team) => format!("{}/{}", team.org, team.slug),
    }
}

/// `reviewers`, spelled and comma-separated.
pub fn spell_list(reviewers: &[Reviewer]) -> String {
    reviewers.iter().map(spell).collect::<Vec<_>>().join(", ")
}

/// Why the sheet's reviewers can't be asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReviewersProblem {
    #[error("no reviewers chosen: name at least one GitHub login or org/team")]
    Empty,
    #[error("{0:?} is not a GitHub login or an org/team")]
    NotAReviewer(String),
    #[error("{0} is listed twice")]
    ListedTwice(String),
}

/// Parses the sheet's text: logins and `org/team`s separated by commas or
/// spaces. Empty, malformed or repeated (case-insensitively) entries are
/// problems, never dropped silently.
pub fn parse_list(text: &str) -> Result<Vec<Reviewer>, ReviewersProblem> {
    let mut reviewers: Vec<Reviewer> = Vec::new();
    for word in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|w| !w.is_empty())
    {
        let reviewer =
            parse(word).ok_or_else(|| ReviewersProblem::NotAReviewer(word.to_owned()))?;
        if reviewers.iter().any(|r| folded(r) == folded(&reviewer)) {
            return Err(ReviewersProblem::ListedTwice(word.to_owned()));
        }
        reviewers.push(reviewer);
    }
    if reviewers.is_empty() {
        return Err(ReviewersProblem::Empty);
    }
    Ok(reviewers)
}

/// A reviewer's identity for duplicate checks: GitHub compares logins and
/// team names case-insensitively.
pub fn folded(reviewer: &Reviewer) -> String {
    spell(reviewer).to_ascii_lowercase()
}

/// One reviewer: a GitHub login, or `org/team-slug`.
pub fn parse(word: &str) -> Option<Reviewer> {
    match word.split_once('/') {
        None => is_valid_login(word).then(|| Reviewer::User(word.to_owned())),
        Some((org, slug)) => (is_valid_login(org) && is_valid_team_slug(slug)).then(|| {
            Reviewer::Team(TeamRef {
                org: org.to_owned(),
                slug: slug.to_owned(),
            })
        }),
    }
}
