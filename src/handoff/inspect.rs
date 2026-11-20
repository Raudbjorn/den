//! Looking at the local workspace for a launch, through the ports only. It
//! reads; it changes nothing (no fetch, no network).

use super::{
    PathState, RegisteredWorktree, WindowMode,
    clone::{self, Candidate, Remote},
    worktree_path,
};
use crate::{
    domain::RepoId,
    jobs::JobRecord,
    ports::{JobStore, Launcher, Workspace},
};
use std::path::PathBuf;

/// What to look at for one launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectRequest {
    pub target: RepoId,
    pub target_url: String,
    /// The Main Clone set for this Repo in the config, if any.
    pub configured_clone: Option<PathBuf>,
    /// Where to search for Main Clones otherwise.
    pub search_root: PathBuf,
    /// The worktree's folder name under `.claude/worktrees/`.
    pub worktree_name: String,
    /// The Job's local branch name.
    pub branch: String,
}

/// How the Main Clone candidates were found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloneSearch {
    /// Set in the config; its remotes, or why they couldn't be read.
    Configured {
        path: PathBuf,
        remotes: Result<Vec<Remote>, String>,
    },
    /// Searched under `root`. Only clones with a remote for the target, or
    /// named like the Repo (near misses), are kept.
    Searched {
        root: PathBuf,
        candidates: Vec<Candidate>,
    },
    SearchFailed {
        root: PathBuf,
        error: String,
    },
}

/// The state of the clone the plan will use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChosenClone {
    pub path: PathBuf,
    pub state: PathState,
    pub worktree: PathState,
    /// The commit of the local branch named like the Job's branch, if it exists.
    pub local_branch: Result<Option<String>, String>,
    /// The worktrees git has registered for the clone.
    pub worktrees: Result<Vec<RegisteredWorktree>, String>,
}

/// Everything local a launch plan depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceView {
    pub clones: CloneSearch,
    /// `None` when no clone qualifies.
    pub chosen: Option<ChosenClone>,
    pub window: WindowMode,
    pub claude_available: bool,
    pub jobs: Result<Vec<JobRecord>, String>,
}

/// Gathers the WorkspaceView for `request`.
pub fn inspect(
    request: &InspectRequest,
    workspace: &dyn Workspace,
    launcher: &dyn Launcher,
    jobs: &dyn JobStore,
) -> WorkspaceView {
    let clones = match &request.configured_clone {
        Some(path) => CloneSearch::Configured {
            path: path.clone(),
            remotes: workspace.remotes(path).map_err(|e| e.to_string()),
        },
        None => search(request, workspace),
    };
    let chosen_path = match &clones {
        CloneSearch::Configured { path, .. } => Some(path.clone()),
        CloneSearch::Searched { candidates, .. } => {
            clone::choose(candidates, &request.target_url, &request.target.name).map(|c| c.path)
        }
        CloneSearch::SearchFailed { .. } => None,
    };
    let chosen = chosen_path.map(|path| ChosenClone {
        state: workspace.path_state(&path),
        worktree: workspace.path_state(&worktree_path(&path, &request.worktree_name)),
        local_branch: workspace
            .resolve(&path, &format!("refs/heads/{}", request.branch))
            .map_err(|e| e.to_string()),
        worktrees: workspace.worktrees(&path).map_err(|e| e.to_string()),
        path,
    });
    WorkspaceView {
        clones,
        chosen,
        window: launcher.window_mode(),
        claude_available: launcher.claude_available(),
        jobs: jobs.load().map_err(|e| e.to_string()),
    }
}

fn search(request: &InspectRequest, workspace: &dyn Workspace) -> CloneSearch {
    let root = request.search_root.clone();
    let paths = match workspace.find_clones(&root) {
        Ok(paths) => paths,
        Err(e) => {
            return CloneSearch::SearchFailed {
                root,
                error: e.to_string(),
            };
        }
    };
    let candidates = paths
        .into_iter()
        .map(|path| Candidate {
            remotes: workspace.remotes(&path).map_err(|e| e.to_string()),
            path,
        })
        .filter(|c| {
            let points_at_target = c
                .remotes
                .as_ref()
                .is_ok_and(|r| clone::remote_for(r, &request.target_url).is_some());
            let named_like_repo = c.path.file_name().is_some_and(|n| {
                n.to_string_lossy()
                    .eq_ignore_ascii_case(&request.target.name)
            });
            points_at_target || named_like_repo
        })
        .collect();
    CloneSearch::Searched { root, candidates }
}
