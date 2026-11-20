//! Refusals: a launch den won't do, with everything that didn't match. den never
//! silently picks another target instead.

use super::{Remote, short_commit};
use crate::domain::RepoId;
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
    /// A Linear Issue: den launches Jobs for it only from the Start and
    /// Implement sheets, which arrive with the Linear writes (#38).
    LinearIssueNotLaunchable,
    /// Request review (#36) is a decision only: its Job needs the reviewer
    /// sheet and the approval-required permission mode, which arrive with #39.
    RequestReviewNotLaunchable,
    /// The Work Item is not in what den last observed.
    WorkItemGone,
    RepoUnknown {
        repo: RepoId,
    },
    RepoArchived {
        repo: RepoId,
    },
    /// The Repo is empty: there is no default branch to start a new branch from.
    NoDefaultBranch {
        repo: RepoId,
    },
    /// The Upstream is the target, but the Work Item's Repo is not a fork (the
    /// stored choice outlived the fork, or `u` was pressed elsewhere).
    NotAFork {
        repo: RepoId,
    },
    /// The Upstream is the target, but den can't see the fork's Upstream.
    UpstreamUnknown {
        fork: RepoId,
    },
    /// A PR is fetched from the Repo it is in; another target (a fork's
    /// Upstream) would fetch a different PR of the same number.
    PullRequestNotInTarget {
        repo: RepoId,
        number: u64,
        target: RepoId,
    },
    /// A Branch is fetched from the Repo it is in; a branch of the same name in
    /// another target (a fork's Upstream) is another branch.
    BranchNotInTarget {
        repo: RepoId,
        branch: String,
        target: RepoId,
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
    /// No Main Clone, and both folders a clone could go into already exist.
    CloneFoldersTaken {
        repo: RepoId,
        folders: Vec<PathBuf>,
    },
    /// No Main Clone, and den can't tell whether the folder it would clone into is free.
    CloneFolderUnknown {
        path: PathBuf,
        error: String,
    },
    /// No Main Clone, and the folder clones go under can't take one.
    CloneRootUnusable {
        root: PathBuf,
        why: String,
    },
    /// The folder den was to clone into appeared after the sheet opened.
    CloneFolderAppeared {
        path: PathBuf,
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
        /// What the base is, in words: "the PR head", "main".
        base_name: String,
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
    RequestNotQueued {
        error: String,
    },
    InspectionFailed {
        error: String,
    },
    /// The head branch is gone from the target's remote.
    HeadBranchGone {
        remote: String,
        branch: String,
    },
    /// The branch a Branch Work Item or a new branch starts from is not on
    /// `from` (a remote, or the URL a clone would come from).
    BaseBranchGone {
        from: String,
        branch: String,
    },
    /// den couldn't ask `from` where the base branch is.
    BaseUnreadable {
        from: String,
        branch: String,
        error: String,
    },
    /// Every suffixed worktree name is taken by other Work Items' Jobs.
    NoFreeWorktreeName {
        wanted: String,
    },
    /// The target's remote has no `refs/pull/<number>/head`.
    PullRefGone {
        remote: String,
        number: u64,
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
            Self::LinearIssueNotLaunchable => f.write_str(
                "den can't launch a Job for a Linear Issue yet: it has no Repo until one is chosen for it, and the sheet that chooses one isn't built yet",
            ),
            Self::RequestReviewNotLaunchable => f.write_str(
                "den can't launch a Request review Job yet: it needs the reviewer sheet and a permission mode where Claude asks before acting, which aren't built yet; request reviewers on GitHub for now",
            ),
            Self::WorkItemGone => f.write_str("this Work Item is no longer in the survey"),
            Self::RepoUnknown { repo } => write!(f, "den has no facts about the Repo {repo}"),
            Self::RepoArchived { repo } => {
                write!(f, "{repo} is archived, so work there can't be pushed")
            }
            Self::NoDefaultBranch { repo } => write!(
                f,
                "{repo} is empty: it has no default branch to start a new branch from"
            ),
            Self::CloneFoldersTaken { repo, folders } => write!(
                f,
                "den won't clone {repo} into a folder that exists: {} {}",
                folders
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" and "),
                if folders.len() == 1 { "exists" } else { "both exist" }
            ),
            Self::CloneFolderUnknown { path, error } => write!(
                f,
                "den can't tell whether {} is free to clone into: {error}",
                path.display()
            ),
            Self::CloneRootUnusable { root, why } => write!(
                f,
                "den can't clone into {}: {why}",
                root.display()
            ),
            Self::CloneFolderAppeared { path } => write!(
                f,
                "{} appeared since the sheet opened; den won't clone into it. Open the sheet again",
                path.display()
            ),
            Self::NotAFork { repo } => write!(
                f,
                "{repo} is not a fork, so it has no Upstream to target; press u to target {repo}"
            ),
            Self::UpstreamUnknown { fork } => write!(
                f,
                "the target is {fork}'s Upstream, but den can't see that Upstream (private, or deleted); press u to target {fork} itself"
            ),
            Self::PullRequestNotInTarget {
                repo,
                number,
                target,
            } => write!(
                f,
                "PR #{number} is in {repo}, not in {target}: den fetches a PR only from the Repo it is in, and #{number} in {target} would be another PR; press u to target {repo} for this Job"
            ),
            Self::BranchNotInTarget {
                repo,
                branch,
                target,
            } => write!(
                f,
                "the Branch {branch} is in {repo}, not in {target}: den fetches a Branch only from the Repo it is in; press u to target {repo} for this Job"
            ),
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
                base_name,
                base,
            } => write!(
                f,
                "a local branch {branch} already exists at {}, not at {base_name} {}; den won't move it",
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
            Self::RequestNotQueued { error } => write!(f, "the request was not queued: {error}; reopen the sheet to retry"),
            Self::InspectionFailed { error } => {
                write!(f, "den couldn't inspect the workspace: {error}")
            }
            Self::HeadBranchGone { remote, branch } => {
                write!(f, "the PR's head branch {branch} is gone from {remote}")
            }
            Self::BaseBranchGone { from, branch } => {
                write!(f, "the branch {branch} is not on {from}")
            }
            Self::BaseUnreadable {
                from,
                branch,
                error,
            } => write!(f, "den couldn't ask {from} where {branch} is: {error}"),
            Self::NoFreeWorktreeName { wanted } => write!(
                f,
                "the worktree name {wanted} and every numbered variant of it are taken by other Work Items' Jobs"
            ),
            Self::PullRefGone { remote, number } => write!(
                f,
                "{remote} has no refs/pull/{number}/head, so the PR's head can't be fetched from it"
            ),
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
