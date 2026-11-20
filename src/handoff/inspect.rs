//! Looking at the local workspace for a launch, through the ports only. It
//! reads; it changes nothing (no fetch). The one network call is asking a
//! remote where a base branch is (`ls-remote`) when GitHub's facts don't say:
//! for Issues, Branches and Runs, not PRs.

use super::{
    PathState, RegisteredWorktree, WindowMode,
    clone::{self, Candidate, Remote},
    cloning::{self, NewCloneView},
    live::{self, JOB_CHECK_BUDGET_SECS, JobCheck},
    names, worktree_path,
};
use crate::{
    domain::{RepoId, WorkItemId},
    jobs::JobRecord,
    ports::{JobStore, Launcher, Workspace},
};
use std::{path::PathBuf, time::Duration};

/// What to look at for one launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectRequest {
    pub target: RepoId,
    pub target_url: String,
    /// The Main Clone set for this Repo in the config, if any.
    pub configured_clone: Option<PathBuf>,
    /// Where to search for Main Clones otherwise.
    pub search_root: PathBuf,
    /// The worktree's folder name under `.claude/worktrees/`, before any suffix.
    pub worktree_name: String,
    /// The Job's local branch name.
    pub branch: String,
    /// The Work Item launched: its own Jobs don't make a name taken (#12),
    /// and its recorded Jobs are checked (#14).
    pub item: WorkItemId,
    /// A new `<type>_<slug>` branch, named like the worktree (suffix included).
    pub new_branch: bool,
    /// Ask the target's remote where this branch is (GitHub's facts don't say).
    pub base_branch: Option<String>,
}

impl InspectRequest {
    /// The same request for a fresh start beside a Live Job (#14): the
    /// worktree folder and the local branch get the suffix `n`
    /// ([`names::suffixed`], the same rule as a name another Work Item's Job has).
    #[must_use]
    pub fn fresh(mut self, n: u32) -> Self {
        self.worktree_name =
            names::suffixed(&self.worktree_name, n, names::MAX_WORKTREE_NAME_CHARS);
        self.branch = names::suffixed(&self.branch, n, names::MAX_BRANCH_NAME_CHARS);
        self
    }
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
    /// The worktree name inspected: the wanted one, or its suffixed variant
    /// when another Work Item's Job has it.
    pub worktree_name: String,
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
    /// Where the base branch is on the target, when den asked: `Ok(None)`
    /// when the target has no such branch.
    pub base_tip: Option<Result<Option<String>, String>>,
    /// When no clone qualifies as Main Clone: the folders a clone could go into.
    pub new_clone: Option<NewCloneView>,
    /// This Work Item's recorded Jobs and their worktrees' states, newest
    /// first (#14); empty when the records couldn't be read.
    pub job_checks: Vec<JobCheck>,
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
    let jobs = jobs.load().map_err(|e| e.to_string());
    let records = jobs.as_ref().map_or(&[][..], Vec::as_slice);
    // The clone, and its remote for the target when it has one.
    let chosen_clone: Option<(PathBuf, Option<Remote>)> = match &clones {
        CloneSearch::Configured { path, remotes } => Some((
            path.clone(),
            remotes
                .as_ref()
                .ok()
                .and_then(|r| clone::remote_for(r, &request.target_url))
                .cloned(),
        )),
        CloneSearch::Searched { candidates, .. } => {
            clone::choose(candidates, &request.target_url, &request.target.name)
                .map(|c| (c.path, Some(c.remote)))
        }
        CloneSearch::SearchFailed { .. } => None,
    };
    let chosen = chosen_clone
        .as_ref()
        .map(|(path, _)| look_at_clone(request, path, records, workspace));
    let new_clone = match (&clones, &chosen) {
        (CloneSearch::Searched { root, .. }, None) => {
            Some(look_for_clone_folders(root, &request.target, workspace))
        }
        _ => None,
    };
    let base_tip = request.base_branch.as_ref().and_then(|branch| {
        let tip = match (&chosen_clone, &new_clone) {
            (Some((path, Some(remote))), _) => {
                workspace.remote_branch_head(path, &remote.name, branch)
            }
            // Only worth asking when den would clone.
            (None, Some(folders)) if cloning::choose(folders, &request.target).is_ok() => {
                workspace.url_branch_head(&request.target_url, branch)
            }
            _ => return None,
        };
        Some(tip.map_err(|e| e.to_string()))
    });
    let job_checks = match &jobs {
        Ok(records) => {
            let mine: Vec<JobRecord> = records
                .iter()
                .filter(|job| job.work_item == request.item)
                .cloned()
                .collect();
            live::check_jobs(&mine, workspace, Duration::from_secs(JOB_CHECK_BUDGET_SECS)).checks
        }
        Err(_) => Vec::new(),
    };
    WorkspaceView {
        clones,
        chosen,
        window: launcher.window_mode(),
        claude_available: launcher.claude_available(),
        jobs,
        base_tip,
        new_clone,
        job_checks,
    }
}

/// The clone's state, with the worktree name suffixed when other Work Items'
/// Jobs have it.
fn look_at_clone(
    request: &InspectRequest,
    path: &std::path::Path,
    records: &[JobRecord],
    workspace: &dyn Workspace,
) -> ChosenClone {
    let worktree_name = names::free_worktree_name(
        &request.worktree_name,
        path,
        &request.item,
        request.new_branch,
        records,
    )
    .map_or_else(|| request.worktree_name.clone(), |free| free.name);
    let branch = if request.new_branch {
        &worktree_name
    } else {
        &request.branch
    };
    ChosenClone {
        state: workspace.path_state(path),
        worktree: workspace.path_state(&worktree_path(path, &worktree_name)),
        local_branch: workspace
            .resolve(path, &format!("refs/heads/{branch}"))
            .map_err(|e| e.to_string()),
        worktrees: workspace.worktrees(path).map_err(|e| e.to_string()),
        path: path.to_path_buf(),
        worktree_name,
    }
}

fn look_for_clone_folders(
    root: &std::path::Path,
    repo: &RepoId,
    workspace: &dyn Workspace,
) -> NewCloneView {
    NewCloneView {
        root: root.to_path_buf(),
        root_state: workspace.path_state(root),
        folders: cloning::folders(root, repo)
            .into_iter()
            .map(|folder| {
                let state = workspace.path_state(&folder);
                (folder, state)
            })
            .collect(),
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
