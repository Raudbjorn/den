//! Finding a Repo's Main Clone (pure): matching a clone's remote URLs against
//! the Repo, and preferring a folder name without a numeric suffix.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A git remote of a clone: its name and fetch URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

/// A local clone den looked at, with its remotes or why they couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub remotes: Result<Vec<Remote>, String>,
}

/// The remote name preferred when several remotes point at the same Repo. Only a
/// tie-break between equally correct remotes: den never assumes it is the target.
const PREFERRED_REMOTE: &str = "origin";

/// A Repo's identity in a URL: host, owner and name, lower-cased (GitHub is
/// case-insensitive), without `.git`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoUrl {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl RepoUrl {
    /// Parses the URL forms git accepts for a hosted Repo:
    /// `https://host/owner/name(.git)`, `ssh://git@host(:port)/owner/name`,
    /// `git://host/owner/name` and scp-like `git@host:owner/name`.
    /// `None` for anything else (local paths, unknown shapes).
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        let (host, path) = match url.split_once("://") {
            Some((_scheme, rest)) => {
                let (authority, path) = rest.split_once('/')?;
                let host = authority.rsplit('@').next()?;
                let host = host.split(':').next()?;
                (host, path)
            }
            None => {
                // scp-like: [user@]host:owner/name; a path with no colon is local.
                let (authority, path) = url.split_once(':')?;
                if authority.contains('/') {
                    return None;
                }
                (authority.rsplit('@').next()?, path)
            }
        };
        let path = path.trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let (owner, name) = path.split_once('/')?;
        if host.is_empty() || owner.is_empty() || name.is_empty() || name.contains('/') {
            return None;
        }
        Some(Self {
            host: host.to_ascii_lowercase(),
            owner: owner.to_ascii_lowercase(),
            name: name.to_ascii_lowercase(),
        })
    }
}

/// Both URLs name the same Repo: the same host, owner and name, or (for URLs
/// that aren't a hosted Repo's, such as local paths) the same location.
///
/// Remotes are compared by the URL git fetches from (after `insteadOf`), so a
/// rewrite to a mirror does not count as the target Repo.
pub fn same_repo(a: &str, b: &str) -> bool {
    match (RepoUrl::parse(a), RepoUrl::parse(b)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => location(a) == location(b),
        _ => false,
    }
}

fn location(url: &str) -> &str {
    let url = url.trim().trim_end_matches('/');
    url.strip_suffix(".git").unwrap_or(url)
}

/// The remote whose URL is the target Repo. When several are, `origin` wins,
/// else the first by name.
pub fn remote_for<'a>(remotes: &'a [Remote], target_url: &str) -> Option<&'a Remote> {
    let mut matching: Vec<&Remote> = remotes
        .iter()
        .filter(|r| same_repo(&r.url, target_url))
        .collect();
    matching.sort_by(|a, b| {
        (a.name != PREFERRED_REMOTE, &a.name).cmp(&(b.name != PREFERRED_REMOTE, &b.name))
    });
    matching.first().copied()
}

/// The folder name ends in a number, like `den-2` or `den2`: probably a copy.
pub fn has_numeric_suffix(folder: &str) -> bool {
    folder.chars().last().is_some_and(|c| c.is_ascii_digit())
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The clone chosen as Main Clone, and the remote in it that points at the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub path: PathBuf,
    pub remote: Remote,
    /// Other clones that also point at the target, for the sheet to mention.
    pub others: Vec<PathBuf>,
}

/// Picks the Main Clone among `candidates` for the Repo at `target_url` named
/// `repo_name`: only clones with a remote whose URL is the Repo qualify; then a
/// folder name without a numeric suffix, then one equal to the Repo's name,
/// then the shallowest path, then the path itself (so the choice is stable).
pub fn choose(candidates: &[Candidate], target_url: &str, repo_name: &str) -> Option<Choice> {
    let mut matching: Vec<(&Candidate, &Remote)> = candidates
        .iter()
        .filter_map(|c| {
            let remotes = c.remotes.as_ref().ok()?;
            Some((c, remote_for(remotes, target_url)?))
        })
        .collect();
    matching.sort_by_key(|(c, _)| {
        let folder = folder_name(&c.path);
        (
            has_numeric_suffix(&folder),
            !folder.eq_ignore_ascii_case(repo_name),
            c.path.components().count(),
            c.path.clone(),
        )
    });
    let mut matching = matching.into_iter();
    let (chosen, remote) = matching.next()?;
    Some(Choice {
        path: chosen.path.clone(),
        remote: remote.clone(),
        others: matching.map(|(c, _)| c.path.clone()).collect(),
    })
}

/// Clones whose folder is named like the Repo but which have no remote for it:
/// the likely "wrong clone" a refusal should name.
pub fn near_misses<'a>(
    candidates: &'a [Candidate],
    target_url: &str,
    repo_name: &str,
) -> impl Iterator<Item = &'a Candidate> {
    candidates.iter().filter(move |c| {
        folder_name(&c.path).eq_ignore_ascii_case(repo_name)
            && c.remotes
                .as_ref()
                .map_or(true, |r| remote_for(r, target_url).is_none())
    })
}
