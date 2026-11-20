//! Carrying out a confirmed LaunchPlan, step by step: re-verify, clone the Main
//! Clone when there is none yet, fetch, add the worktree, open the window (or
//! copy the command), record the Job. It stops at
//! the first failure and reports what was already done. It never undoes an
//! earlier step by deleting anything.

use super::{
    BranchStart, CloneSource, LaunchPlan, PathState, Refusal, RefusalReason, WindowMode,
    check_prompt, clone, command, short_commit,
};
use crate::{
    jobs::JobRecord,
    ports::{JobStore, Launcher, Workspace, WorktreeAdd, WorktreeStart},
};
use jiff::Timestamp;
use std::fmt;

/// The steps of a launch, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStep {
    Verify,
    Clone,
    Fetch,
    AddWorktree,
    OpenWindow,
    CopyCommand,
    RecordJob,
}

impl fmt::Display for LaunchStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Verify => "re-verify",
            Self::Clone => "clone the Repo",
            Self::Fetch => "fetch",
            Self::AddWorktree => "add the worktree",
            Self::OpenWindow => "open the kitty window",
            Self::CopyCommand => "copy the command to the clipboard",
            Self::RecordJob => "record the Job",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOutcome {
    /// Every step ran.
    Launched { job: JobRecord },
    /// Re-verification found a mismatch; nothing was done.
    Refused(Refusal),
    /// The base (a PR head, a Branch, the default branch) moved since the sheet
    /// showed it; nothing was done. The sheet asks again on the new commit.
    HeadMoved { from: String, to: String },
    /// `step` failed; the steps in `done` had already run and were left as they are.
    Failed { step: LaunchStep, error: String },
}

/// What a launch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReport {
    /// The steps that completed, in words, in order.
    pub done: Vec<String>,
    pub outcome: LaunchOutcome,
}

/// Carries `plan` out through the ports. `now` is the Job's launch time.
pub fn execute(
    plan: &LaunchPlan,
    workspace: &dyn Workspace,
    launcher: &dyn Launcher,
    jobs: &dyn JobStore,
    now: Timestamp,
) -> LaunchReport {
    let mut done = Vec::new();
    let outcome = run(plan, workspace, launcher, jobs, now, &mut done);
    LaunchReport { done, outcome }
}

fn run(
    plan: &LaunchPlan,
    workspace: &dyn Workspace,
    launcher: &dyn Launcher,
    jobs: &dyn JobStore,
    now: Timestamp,
    done: &mut Vec<String>,
) -> LaunchOutcome {
    let failed = |step: LaunchStep, error: String| LaunchOutcome::Failed { step, error };
    let clone_path = &plan.main_clone.path;

    // 1. Re-verify, changing nothing.
    match verify(plan, workspace) {
        Ok(()) if plan.clones() => done.push(format!(
            "re-verified: {} is free; {} has {} at {}",
            plan.main_clone.path.display(),
            plan.target.repo,
            plan.base.reference.branch(),
            short_commit(&plan.base.commit)
        )),
        Ok(()) => done.push(format!(
            "re-verified: {} → {} at {}; the worktree path is free",
            plan.remote.name,
            plan.target.repo,
            short_commit(&plan.base.commit)
        )),
        Err(Verification::Refused(reason)) => return LaunchOutcome::Refused(Refusal::one(reason)),
        Err(Verification::Moved(to)) => {
            return LaunchOutcome::HeadMoved {
                from: plan.base.commit.clone(),
                to,
            };
        }
        Err(Verification::Failed(error)) => return failed(LaunchStep::Verify, error),
    }

    // 2. Clone the Main Clone when there is none yet.
    if let CloneSource::ToClone { url, .. } = &plan.main_clone.source {
        if let Err(e) = workspace.clone_repo(url, &plan.remote.name, clone_path) {
            return failed(LaunchStep::Clone, e.to_string());
        }
        done.push(format!("cloned {url} into {}", clone_path.display()));
    }

    // 3. Fetch from the remote whose URL is the target, and check what arrived.
    let refspec = plan.fetch_refspec();
    if let Err(e) = workspace.fetch(clone_path, &plan.remote.name, &refspec) {
        return failed(LaunchStep::Fetch, e.to_string());
    }
    let tracking = plan.tracking_ref();
    match workspace.resolve(clone_path, &tracking) {
        Ok(Some(commit)) if commit == plan.base.commit => done.push(format!(
            "fetched {tracking} from {} ({})",
            plan.remote.name,
            short_commit(&commit)
        )),
        Ok(found) => {
            return failed(
                LaunchStep::Fetch,
                format!(
                    "after fetching, {tracking} is at {}, not the verified {}",
                    found.as_deref().map_or("nothing", short_commit),
                    short_commit(&plan.base.commit)
                ),
            );
        }
        Err(e) => return failed(LaunchStep::Fetch, e.to_string()),
    }

    // 4. Add the worktree with git itself (not Claude Code's worktree options).
    let start = match &plan.branch.start {
        BranchStart::New { .. } => WorktreeStart::NewBranchTracking { start: tracking },
        BranchStart::Existing => WorktreeStart::ExistingBranch,
        BranchStart::Fresh => WorktreeStart::NewBranch { start: tracking },
    };
    let add = WorktreeAdd {
        path: plan.worktree.clone(),
        branch: plan.branch.name.clone(),
        start,
    };
    if let Err(e) = workspace.add_worktree(clone_path, &add) {
        return failed(LaunchStep::AddWorktree, e.to_string());
    }
    done.push(format!(
        "added the worktree {} on branch {}",
        plan.worktree.display(),
        plan.branch.name
    ));

    // 5. Open the window, or put the command on the clipboard.
    let window = plan.window_command();
    match &plan.window {
        WindowMode::Kitty => {
            if let Err(e) = launcher.open_window(&window) {
                return failed(LaunchStep::OpenWindow, e.to_string());
            }
            done.push(format!(
                "opened a kitty window running claude -n {}",
                plan.session
            ));
        }
        WindowMode::Clipboard { .. } => {
            if let Err(e) = launcher.copy_to_clipboard(&command::clipboard_text(&window)) {
                return failed(LaunchStep::CopyCommand, e.to_string());
            }
            done.push("copied the command to the clipboard".to_owned());
        }
    }

    // 6. Record the Job.
    let job = JobRecord {
        work_item: plan.work_item.clone(),
        next_step: plan.next_step,
        target: plan.target.repo.clone(),
        main_clone: clone_path.clone(),
        worktree: plan.worktree.clone(),
        branch: plan.branch.name.clone(),
        session: plan.session.clone(),
        launched_at: now,
    };
    if let Err(e) = jobs.record(&job) {
        return failed(LaunchStep::RecordJob, e.to_string());
    }
    done.push("recorded the Job".to_owned());
    LaunchOutcome::Launched { job }
}

enum Verification {
    Refused(RefusalReason),
    Moved(String),
    Failed(String),
}

/// Just before anything changes: the Main Clone still has a remote whose URL is
/// the target, the worktree path is free, the local branch is as planned, the
/// prompt is passable, and the base is still the commit the sheet showed. For
/// a clone den is about to make: its folder is still free and the target's
/// base branch is still where the sheet said.
fn verify(plan: &LaunchPlan, workspace: &dyn Workspace) -> Result<(), Verification> {
    let clone_path = &plan.main_clone.path;
    check_prompt(&plan.prompt).map_err(|e| Verification::Failed(e.to_string()))?;
    if let CloneSource::ToClone { url, .. } = &plan.main_clone.source {
        return verify_new_clone(plan, workspace, url);
    }
    let remotes = workspace
        .remotes(clone_path)
        .map_err(|e| Verification::Failed(e.to_string()))?;
    let still_points_at_target = remotes
        .iter()
        .any(|r| r.name == plan.remote.name && clone::same_repo(&r.url, &plan.target.url));
    if !still_points_at_target {
        return Err(Verification::Refused(RefusalReason::NoRemoteForTarget {
            clone: clone_path.clone(),
            target: plan.target.repo.clone(),
            remotes,
        }));
    }
    match workspace.path_state(&plan.worktree) {
        PathState::Missing => {}
        PathState::Exists { .. } => {
            return Err(Verification::Refused(RefusalReason::WorktreeExists {
                path: plan.worktree.clone(),
                job: None,
            }));
        }
        PathState::Unknown(error) => {
            return Err(Verification::Refused(RefusalReason::WorktreeUnknown {
                path: plan.worktree.clone(),
                error,
            }));
        }
    }
    let local = workspace
        .resolve(clone_path, &format!("refs/heads/{}", plan.branch.name))
        .map_err(|e| Verification::Failed(e.to_string()))?;
    let as_planned = match (&plan.branch.start, &local) {
        (BranchStart::New { .. } | BranchStart::Fresh, None) => true,
        (BranchStart::Existing, Some(commit)) => *commit == plan.base.commit,
        _ => false,
    };
    if !as_planned {
        return Err(Verification::Refused(RefusalReason::LocalBranchChanged {
            branch: plan.branch.name.clone(),
        }));
    }
    let branch = plan.base.reference.branch();
    let tip = workspace
        .remote_branch_head(clone_path, &plan.remote.name, branch)
        .map_err(|e| Verification::Failed(e.to_string()))?;
    still_at_base(plan, tip, &plan.remote.name)
}

fn verify_new_clone(
    plan: &LaunchPlan,
    workspace: &dyn Workspace,
    url: &str,
) -> Result<(), Verification> {
    let folder = &plan.main_clone.path;
    match workspace.path_state(folder) {
        PathState::Missing => {}
        PathState::Exists { .. } => {
            return Err(Verification::Refused(RefusalReason::CloneFolderAppeared {
                path: folder.clone(),
            }));
        }
        PathState::Unknown(error) => {
            return Err(Verification::Refused(RefusalReason::CloneFolderUnknown {
                path: folder.clone(),
                error,
            }));
        }
    }
    let tip = workspace
        .url_branch_head(url, plan.base.reference.branch())
        .map_err(|e| Verification::Failed(e.to_string()))?;
    still_at_base(plan, tip, url)
}

/// The base branch's tip (as `from` reports it) is the commit the sheet showed.
fn still_at_base(plan: &LaunchPlan, tip: Option<String>, from: &str) -> Result<(), Verification> {
    let branch = plan.base.reference.branch().to_owned();
    match tip {
        None if plan.work_item.kind() == crate::domain::WorkItemKind::PullRequest => {
            Err(Verification::Refused(RefusalReason::HeadBranchGone {
                remote: from.to_owned(),
                branch,
            }))
        }
        None => Err(Verification::Refused(RefusalReason::BaseBranchGone {
            from: from.to_owned(),
            branch,
        })),
        Some(commit) if commit == plan.base.commit => Ok(()),
        Some(commit) => Err(Verification::Moved(commit)),
    }
}
