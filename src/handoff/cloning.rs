//! Cloning a Repo that has no Main Clone (pure; spec, "Launch handoff", user
//! story 79): into `<root>/<repo>`, or `<root>/<owner>-<repo>` when that is
//! taken, and never into a folder that exists. `inspect` looks at the folders;
//! this decides which one, or refuses.

use super::{PathState, RefusalReason, Writability, names};
use crate::domain::RepoId;
use std::path::{Path, PathBuf};

/// The name git gives the remote a clone was made from; den clones from the
/// target Repo's URL, so in a clone den made this remote is the target.
pub const CLONE_REMOTE: &str = "origin";

/// The folders `repo` may be cloned into, best first: `<root>/<repo>`, then
/// `<root>/<owner>-<repo>`. Names are cleaned like worktree names, so they
/// stay inside `root` and are never hidden (the Main Clone search skips
/// hidden folders, so a clone there would never be found again).
pub fn folders(root: &Path, repo: &RepoId) -> [PathBuf; 2] {
    [
        root.join(names::worktree_name(&repo.name)),
        root.join(names::worktree_name(&format!(
            "{}-{}",
            repo.owner, repo.name
        ))),
    ]
}

/// What `inspect` saw where a clone could go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCloneView {
    pub root: PathBuf,
    pub root_state: PathState,
    /// Each folder from [`folders`], in order, with what is there.
    pub folders: Vec<(PathBuf, PathState)>,
}

/// Where the clone goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneSpot {
    pub path: PathBuf,
    /// Folders passed over because something is there.
    pub taken: Vec<PathBuf>,
    /// Whether den may write in the root the clone goes under.
    pub writable: Writability,
}

/// The first free folder, or why den won't clone. A folder den can't inspect
/// stops the choice there: it is never treated as missing, and never skipped
/// for the next one.
pub fn choose(view: &NewCloneView, repo: &RepoId) -> Result<CloneSpot, RefusalReason> {
    let unusable = |why: String| RefusalReason::CloneRootUnusable {
        root: view.root.clone(),
        why,
    };
    let writable = match &view.root_state {
        PathState::Exists {
            writable: Writability::NotWritable,
        } => return Err(unusable("den can't write in it".to_owned())),
        PathState::Exists { writable } => writable.clone(),
        PathState::Missing => return Err(unusable("it doesn't exist".to_owned())),
        PathState::Unknown(error) => {
            return Err(unusable(format!("den can't inspect it: {error}")));
        }
    };
    let mut taken = Vec::new();
    for (path, state) in &view.folders {
        match state {
            PathState::Missing => {
                return Ok(CloneSpot {
                    path: path.clone(),
                    taken,
                    writable,
                });
            }
            PathState::Exists { .. } => taken.push(path.clone()),
            PathState::Unknown(error) => {
                return Err(RefusalReason::CloneFolderUnknown {
                    path: path.clone(),
                    error: error.clone(),
                });
            }
        }
    }
    Err(RefusalReason::CloneFoldersTaken {
        repo: repo.clone(),
        folders: taken,
    })
}
