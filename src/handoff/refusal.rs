//! Refusals: a launch den won't do, with everything that didn't match. den never
//! silently picks another target instead.

use super::{Remote, short_commit};
use crate::domain::{RepoId, WorkItemKind};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub reasons: Vec<RefusalReason>,
}

impl Refusal {
    pub fn one(reason: RefusalReason) -> Self {
        Self {
            reasons: vec![reason],
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reasons: Vec<String> = self.reasons.iter().map(ToString::to_string).collect();
        f.write_str(&reasons.join("; "))
    }
}

/// One thing that didn't match. Each says what, in words a user can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefusalReason {
    NoNextStep,
    /// Launching for this kind of Work Item arrives later (#12).
    NotYetLaunchable {
        kind: WorkItemKind,
    },
    /// The Work Item is not in what den last observed.
    WorkItemGone,
    RepoUnknown {
        repo: RepoId,
    },
    RepoArchived {
        repo: RepoId,
    },
    /// The PR's head branch lives in another Repo (#13), or that Repo was deleted.
    CrossRepoHead {
        head_repo: Option<RepoId>,
    },
    PromptsUnusable {
        error: String,
    },
    JobRecordsUnreadable {
        error: String,
    },
    /// No `main_clone_root` and no default to search.
    NoSearchRoot,
    NoMainClone {
        repo: RepoId,
        root: PathBuf,
        /// Why the search itself failed, if it did.
        search_error: Option<String>,
        /// Clones named like the Repo whose remotes point elsewhere.
        near_misses: Vec<(PathBuf, Vec<Remote>)>,
    },
    ConfiguredCloneUnreadable {
        path: PathBuf,
        error: String,
    },
    /// The Main Clone has no remote whose URL is the target Repo.
    NoRemoteForTarget {
        clone: PathBuf,
        target: RepoId,
        remotes: Vec<Remote>,
    },
    MainCloneMissing {
        path: PathBuf,
    },
    MainCloneNotWritable {
        path: PathBuf,
    },
    MainCloneUnknown {
        path: PathBuf,
        error: String,
    },
    WorktreeExists {
        path: PathBuf,
        /// The Job recorded there, when den launched it. A Live Job there is
        /// asked about before planning (#14), so this is the leftover case.
        job: Option<(String, Timestamp)>,
    },
    WorktreeUnknown {
        path: PathBuf,
        error: String,
    },
    /// A local branch of that name exists at another commit; den won't move it.
    LocalBranchDiffers {
        branch: String,
        commit: String,
        base: String,
    },
    /// The local branch is already checked out in another worktree; git checks
    /// a branch out in one worktree at a time.
    BranchCheckedOut {
        branch: String,
        worktree: PathBuf,
    },
    /// The worktree path is gone from disk but git still has it registered.
    WorktreeRegistered {
        path: PathBuf,
    },
    /// A local branch of that name appeared (or vanished) since the sheet opened.
    LocalBranchChanged {
        branch: String,
    },
    InspectionFailed {
        error: String,
    },
    /// The head branch is gone from the target's remote.
    HeadBranchGone {
        remote: String,
        branch: String,
    },
    ClaudeMissing,
    /// Open existing (#14): the Job's worktree is not (or no longer) present.
    JobNotLive {
        worktree: PathBuf,
        why: String,
    },
    /// Open existing (#14): the recorded worktree is not inside the Main
    /// Clone's `.claude/worktrees/`, so it could be the Main Clone itself.
    ReopenOutsideWorktrees {
        worktree: PathBuf,
        main_clone: PathBuf,
    },
    /// Open existing (#14): the recorded session name isn't one den makes.
    SessionNameUnusable {
        session: String,
    },
    /// Start fresh (#14): every suffixed name up to the limit is taken.
    NoFreshName {
        last_tried: u32,
    },
}

impl fmt::Display for RefusalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoNextStep => f.write_str("this Work Item has no Next Step to hand off"),
            Self::NotYetLaunchable { kind } => write!(
                f,
                "launching a Job for {} isn't built yet (only Pull Requests so far)",
                kind_plural(*kind)
            ),
            Self::WorkItemGone => f.write_str("this Work Item is no longer in the survey"),
            Self::RepoUnknown { repo } => write!(f, "den has no facts about the Repo {repo}"),
            Self::RepoArchived { repo } => {
                write!(f, "{repo} is archived, so work there can't be pushed")
            }
            Self::CrossRepoHead { head_repo: Some(head) } => write!(
                f,
                "the PR's head branch lives in another Repo ({head}); launching for cross-Repo heads isn't built yet"
            ),
            Self::CrossRepoHead { head_repo: None } => {
                f.write_str("the Repo holding the PR's head branch was deleted")
            }
            Self::PromptsUnusable { error } => write!(f, "the prompts file is unusable: {error}"),
            Self::JobRecordsUnreadable { error } => write!(
                f,
                "den can't read its Job records, so it can't tell whether this Work Item already has a Job: {error}"
            ),
            Self::NoSearchRoot => f.write_str(
                "den doesn't know where to look for Main Clones: set main_clone_root in config.toml",
            ),
            Self::NoMainClone {
                repo,
                root,
                search_error,
                near_misses,
            } => {
                write!(
                    f,
                    "no clone under {} has a remote for {repo}",
                    root.display()
                )?;
                if let Some(error) = search_error {
                    write!(f, " (the search failed: {error})")?;
                }
                for (path, remotes) in near_misses {
                    write!(
                        f,
                        "; {} is named like it but its remotes are {}",
                        path.display(),
                        remotes_text(remotes)
                    )?;
                }
                write!(f, "; set its Main Clone under [main_clones] in config.toml")
            }
            Self::ConfiguredCloneUnreadable { path, error } => write!(
                f,
                "den can't read the remotes of the configured Main Clone {}: {error}",
                path.display()
            ),
            Self::NoRemoteForTarget {
                clone,
                target,
                remotes,
            } => write!(
                f,
                "the Main Clone {} has no remote for {target}; its remotes are {}",
                clone.display(),
                remotes_text(remotes)
            ),
            Self::MainCloneMissing { path } => {
                write!(f, "the Main Clone {} doesn't exist", path.display())
            }
            Self::MainCloneNotWritable { path } => {
                write!(f, "den can't write in the Main Clone {}", path.display())
            }
            Self::MainCloneUnknown { path, error } => write!(
                f,
                "den can't inspect the Main Clone {}: {error}",
                path.display()
            ),
            Self::WorktreeExists { path, job: None } => write!(
                f,
                "the worktree path {} already exists and isn't a Job den launched",
                path.display()
            ),
            Self::WorktreeExists {
                path,
                job: Some((session, at)),
            } => write!(
                f,
                "a Job for this Work Item (session {session}, launched {}) already has the worktree {}, but den couldn't confirm git has it registered",
                at.strftime("%Y-%m-%d %H:%M UTC"),
                path.display()
            ),
            Self::WorktreeUnknown { path, error } => write!(
                f,
                "den can't tell whether the worktree path {} is free: {error}",
                path.display()
            ),
            Self::LocalBranchDiffers {
                branch,
                commit,
                base,
            } => write!(
                f,
                "a local branch {branch} already exists at {}, not at the PR head {}; den won't move it",
                short_commit(commit),
                short_commit(base)
            ),
            Self::BranchCheckedOut { branch, worktree } => write!(
                f,
                "the local branch {branch} is already checked out in {}; git checks a branch out in one worktree at a time",
                worktree.display()
            ),
            Self::WorktreeRegistered { path } => write!(
                f,
                "git still has a worktree registered at {} though its folder is gone; den doesn't prune worktrees (git worktree prune would)",
                path.display()
            ),
            Self::LocalBranchChanged { branch } => write!(
                f,
                "the local branch {branch} changed since the sheet opened; open the sheet again"
            ),
            Self::InspectionFailed { error } => {
                write!(f, "den couldn't inspect the workspace: {error}")
            }
            Self::HeadBranchGone { remote, branch } => {
                write!(f, "the PR's head branch {branch} is gone from {remote}")
            }
            Self::ClaudeMissing => f.write_str("Claude Code (claude) is not on PATH"),
            Self::JobNotLive { worktree, why } => write!(
                f,
                "the Job's worktree {} is not a Live Job's any more ({why}); den leaves it as it is",
                worktree.display()
            ),
            Self::ReopenOutsideWorktrees {
                worktree,
                main_clone,
            } => write!(
                f,
                "the recorded worktree {} isn't inside {}/.claude/worktrees; den never reopens a Job outside it (and never in the Main Clone itself)",
                worktree.display(),
                main_clone.display()
            ),
            Self::SessionNameUnusable { session } => write!(
                f,
                "the recorded session name {session:?} isn't one den makes, so den won't pass it to claude --resume"
            ),
            Self::NoFreshName { last_tried } => write!(
                f,
                "every fresh worktree name up to -{last_tried} is taken by a Job or a worktree; den deletes none of them"
            ),
        }
    }
}

fn kind_plural(kind: WorkItemKind) -> &'static str {
    match kind {
        WorkItemKind::Issue => "Issues",
        WorkItemKind::PullRequest => "Pull Requests",
        WorkItemKind::Branch => "Branches",
        WorkItemKind::Run => "Runs",
    }
}

fn remotes_text(remotes: &[Remote]) -> String {
    if remotes.is_empty() {
        return "none".to_owned();
    }
    remotes
        .iter()
        .map(|r| format!("{} → {}", r.name, r.url))
        .collect::<Vec<_>>()
        .join(", ")
}
