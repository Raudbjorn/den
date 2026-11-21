//! Where den starts (#90; spec 0003, "Screens and navigation", Startup). Pure:
//! the git read is passed in, and only run when the rules need it.
//!
//! The first rule that matches wins:
//!
//! 1. `den --pick` chooses no Repo, even inside a clone.
//! 2. `den OWNER/REPO` opens that Repo.
//! 3. Inside a git work tree (a Job worktree reads as its Main Clone,
//!    [`crate::ports::Workspace::repo_at`]): a GitHub `origin` opens its Repo;
//!    otherwise the only GitHub Repo among the remotes opens; with several and
//!    no GitHub `origin`, den doesn't guess and names the candidates.
//! 4. Otherwise no Repo is chosen. A git work tree without a GitHub remote, or
//!    a directory den couldn't read, says so in a notice.
//!
//! Until milestone 3's Repo screen exists, "no Repo chosen" opens All Owners.

use crate::{
    domain::RepoId,
    handoff::{Remote, clone::RepoUrl},
    ports::WorkspaceError,
};
use std::path::PathBuf;

/// The only host whose Repos den surveys (GitHub Enterprise is out of scope).
pub const GITHUB_HOST: &str = "github.com";
/// The remote that wins when it names a GitHub Repo.
const ORIGIN: &str = "origin";

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StartArg {
    /// No argument: follow the directory den was started in.
    #[default]
    Auto,
    /// `--pick`: choose no Repo.
    Pick,
    /// `OWNER/REPO`.
    Repo(RepoId),
}

/// What git says about the directory den was started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitDir {
    /// Not inside a git work tree (including inside a `.git` folder).
    NotAWorkTree,
    /// Inside a work tree: its Main Clone (a Job worktree's is the clone it
    /// was added from) and the remotes, with their fetch URLs.
    WorkTree {
        main_clone: PathBuf,
        remotes: Vec<Remote>,
    },
}

/// Where den opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Repo(RepoId),
    /// No Repo chosen, and why when it matters.
    Unchosen {
        notice: Option<String>,
    },
}

impl Default for Start {
    fn default() -> Self {
        Self::Unchosen { notice: None }
    }
}

/// The first start rule that matches. `read_dir` is called only when the
/// command line names nothing.
pub fn resolve(arg: &StartArg, read_dir: impl FnOnce() -> Result<GitDir, WorkspaceError>) -> Start {
    match arg {
        StartArg::Pick => Start::default(),
        StartArg::Repo(repo) => Start::Repo(repo.clone()),
        StartArg::Auto => match read_dir() {
            Ok(GitDir::NotAWorkTree) => Start::default(),
            Ok(GitDir::WorkTree {
                main_clone,
                remotes,
            }) => from_remotes(&main_clone, &remotes),
            Err(error) => Start::Unchosen {
                notice: Some(format!(
                    "den couldn't tell which Repo it was started in ({error})"
                )),
            },
        },
    }
}

/// Rule 3 and the notice of rule 4.
fn from_remotes(main_clone: &std::path::Path, remotes: &[Remote]) -> Start {
    if let Some(origin) = remotes
        .iter()
        .find(|r| r.name == ORIGIN)
        .and_then(|r| github_repo(&r.url))
    {
        return Start::Repo(origin);
    }
    let mut candidates: Vec<(RepoId, &str)> = Vec::new();
    for remote in remotes {
        if let Some(repo) = github_repo(&remote.url)
            && !candidates.iter().any(|(known, _)| known.same(&repo))
        {
            candidates.push((repo, &remote.name));
        }
    }
    match candidates.as_slice() {
        [] => Start::Unchosen {
            notice: Some(format!(
                "{} has no GitHub remote, so den didn't open a Repo",
                main_clone.display()
            )),
        },
        [(only, _)] => Start::Repo(only.clone()),
        several => Start::Unchosen {
            notice: Some(format!(
                "{} has several GitHub remotes and no GitHub origin, so den didn't choose: {}; open one with `den OWNER/REPO`",
                main_clone.display(),
                several
                    .iter()
                    .map(|(repo, remote)| format!("{repo} ({remote})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        },
    }
}

/// The GitHub Repo a remote URL names, spelled as in the URL; `None` for
/// another host or a URL that isn't a hosted Repo's.
pub fn github_repo(url: &str) -> Option<RepoId> {
    let parsed = RepoUrl::parse(url)?;
    if parsed.host != GITHUB_HOST {
        return None;
    }
    // `RepoUrl` lower-cases (GitHub compares names that way); the URL's own
    // spelling reads better in the header, and is the same Repo.
    let path = url.trim().trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segments = path.rsplit(['/', ':']);
    let (name, owner) = (segments.next()?, segments.next()?);
    if owner.eq_ignore_ascii_case(&parsed.owner) && name.eq_ignore_ascii_case(&parsed.name) {
        Some(RepoId::new(owner, name))
    } else {
        Some(RepoId::new(parsed.owner, parsed.name))
    }
}
