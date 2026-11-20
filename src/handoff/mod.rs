//! The launch handoff (spec, "Launch handoff"): hands a Work Item's Next Step to
//! Claude Code as a Job, in its own worktree and kitty window.
//!
//! - [`inspect`] looks at the local workspace through the ports (Main Clone
//!   candidates and their remotes, the worktree path, the local branch, kitty or
//!   not, Job records) and returns a [`WorkspaceView`].
//! - [`plan`] is pure: from the Decision, the Observation, the target choice,
//!   the WorkspaceView (which carries the Job records) and the prompts it gives
//!   a [`LaunchPlan`] or a [`Refusal`] naming everything that didn't match.
//! - [`execute`] carries a confirmed plan out: re-verify, fetch from the remote
//!   whose URL is the target Repo, add the worktree, open the window (or copy
//!   the command), record the Job. It stops at the first failure, reports what
//!   was done, and never deletes anything.
//!
//! Extension points for later tickets:
//! - #12 (Issues, Branches, Runs; cloning; worktree-name suffixes):
//!   `plan::facts` refuses non-PR kinds with [`RefusalReason::NotYetLaunchable`];
//!   add their facts there and a `BaseRef::DefaultBranch` start. A missing Main
//!   Clone is [`RefusalReason::NoMainClone`]; cloning becomes an executor step
//!   before `Fetch`. Name collisions are [`RefusalReason::WorktreeExists`]
//!   today, with the Job records already in the WorkspaceView.
//! - #13 (Upstream target; cross-Repo heads): [`TargetChoice`] gains `Upstream`;
//!   [`RefusalReason::CrossRepoHead`] becomes a `BaseRef::PullHead` fetched as
//!   `refs/pull/<n>/head` from the target's remote.
//! - #14 (Live Jobs, built): [`live`] tells each recorded Job's worktree
//!   present / missing / unknown. The WorkspaceView carries this Work Item's
//!   checks (`job_checks`); the App asks "open existing or start fresh?" on a
//!   Live Job. Open existing is a [`live::ReopenPlan`] carried out by
//!   [`live::reopen`] (`claude --resume`, never in the Main Clone); start fresh
//!   is [`PlanInput::fresh`], a suffix on the worktree name, branch and
//!   session ([`names::suffixed`]).

pub mod clone;
pub mod command;
mod execute;
mod inspect;
pub mod live;
pub mod names;
mod plan;
pub mod prompts;
mod refusal;

pub use clone::Remote;
pub use command::{SessionStart, WindowCommand, WindowMode};
pub use execute::{LaunchOutcome, LaunchReport, LaunchStep, execute};
pub use inspect::{ChosenClone, CloneSearch, InspectRequest, WorkspaceView, inspect};
pub use live::{JobCheck, JobsCheck, ReopenPlan, WorktreeState};
pub use plan::{LaunchSettings, PlanInput, inspect_request, plan};
pub use refusal::{Refusal, RefusalReason};

use crate::domain::{NextStep, RepoId, WorkItemId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where Jobs' worktrees live inside a Main Clone (where Claude Code expects them).
pub const WORKTREES_DIR: [&str; 2] = [".claude", "worktrees"];
/// The longest prompt den passes to Claude Code, in characters.
pub const MAX_PROMPT_CHARS: usize = 8_000;
/// Characters of a commit shown on the sheet.
pub const SHORT_COMMIT_CHARS: usize = 7;

/// The worktree path for a Job named `name` in `clone`.
pub fn worktree_path(clone: &Path, name: &str) -> PathBuf {
    WORKTREES_DIR
        .iter()
        .fold(clone.to_path_buf(), |path, part| path.join(part))
        .join(name)
}

/// A commit, shortened for display.
pub fn short_commit(commit: &str) -> &str {
    commit.get(..SHORT_COMMIT_CHARS).unwrap_or(commit)
}

/// Whether den may write in a directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Writability {
    Writable,
    NotWritable,
    /// den couldn't tell (for example, writable only by a group).
    Unknown(String),
}

/// What is at a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathState {
    Missing,
    Exists {
        writable: Writability,
    },
    /// den couldn't inspect it; never treated as missing.
    Unknown(String),
}

/// A worktree git has registered for a clone (`git worktree list`). #14 reads
/// `locked` and `prunable` to tell a Live Job's worktree present / missing / unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredWorktree {
    pub path: PathBuf,
    /// The branch checked out there; `None` when detached (or bare).
    pub branch: Option<String>,
    pub locked: bool,
    /// git would prune it: its folder is gone.
    pub prunable: bool,
}

/// Which Repo the Job's work goes to. #13 adds `Upstream` (a fork's parent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetChoice {
    /// The Work Item's own Repo (for a fork: the fork itself).
    #[default]
    OwnRepo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub repo: RepoId,
    /// The Repo's URL on GitHub; a remote matches it when both name the same Repo.
    pub url: String,
    pub is_fork: bool,
}

/// How the Main Clone was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloneSource {
    /// By matching remotes under `root`; `others` also matched and lost.
    Searched { root: PathBuf, others: Vec<PathBuf> },
    /// From `[main_clones]` in the config.
    Configured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainClone {
    pub path: PathBuf,
    pub source: CloneSource,
    pub writable: Writability,
}

/// What the Job's worktree starts from. #12 adds the target's default branch;
/// #13 adds a cross-Repo PR head (fetched as the PR ref).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseRef {
    /// A same-Repo PR's head branch, fetched by name.
    HeadBranch(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    pub reference: BaseRef,
    /// The commit the sheet showed, and the worktree will start on.
    pub commit: String,
}

/// The Job's local branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchStart {
    /// Created at the base, tracking `tracking` (`<remote>/<branch>`).
    New { tracking: String },
    /// Already exists locally at the base commit; checked out as is.
    Existing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobBranch {
    pub name: String,
    pub start: BranchStart,
}

/// Everything a launch will do, decided before anything is done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchPlan {
    pub work_item: WorkItemId,
    pub title: String,
    pub url: String,
    pub next_step: NextStep,
    pub target: Target,
    pub main_clone: MainClone,
    /// The remote whose URL is the target Repo (not assumed to be `origin`).
    pub remote: Remote,
    pub base: Base,
    pub branch: JobBranch,
    pub worktree: PathBuf,
    pub session: String,
    pub prompt: String,
    pub window: WindowMode,
    /// The exact command, as shown on the sheet.
    pub command: String,
}

/// Why a prompt can't be passed to Claude Code as is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromptProblem {
    #[error("the prompt is empty")]
    Empty,
    #[error("the prompt starts with '-', which Claude Code would read as an option")]
    LooksLikeAnOption,
    #[error("the prompt is longer than {MAX_PROMPT_CHARS} characters")]
    TooLong,
    #[error("the prompt contains a NUL character")]
    HasNul,
}

/// Checks `prompt` can be passed as Claude Code's prompt argument.
pub fn check_prompt(prompt: &str) -> Result<(), PromptProblem> {
    if prompt.trim().is_empty() {
        return Err(PromptProblem::Empty);
    }
    if prompt.starts_with('-') {
        return Err(PromptProblem::LooksLikeAnOption);
    }
    if prompt.contains('\0') {
        return Err(PromptProblem::HasNul);
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(PromptProblem::TooLong);
    }
    Ok(())
}

impl LaunchPlan {
    /// What the Job's window runs.
    pub fn window_command(&self) -> WindowCommand {
        WindowCommand {
            directory: self.worktree.clone(),
            session: self.session.clone(),
            prompt: self.prompt.clone(),
            start: SessionStart::New,
        }
    }

    /// The plan with `prompt` instead (the sheet's edited text), its command
    /// rebuilt to match.
    pub fn with_prompt(mut self, prompt: &str) -> Result<Self, PromptProblem> {
        check_prompt(prompt)?;
        self.prompt = prompt.to_owned();
        self.command = command::display(&self.window, &self.window_command());
        Ok(self)
    }

    /// The plan on `commit` instead (the PR head moved and the user accepted it).
    pub fn on_commit(mut self, commit: &str) -> Self {
        commit.clone_into(&mut self.base.commit);
        self
    }

    /// The remote-tracking ref the base is fetched into: `refs/remotes/<remote>/<branch>`.
    pub fn tracking_ref(&self) -> String {
        match &self.base.reference {
            BaseRef::HeadBranch(branch) => format!("refs/remotes/{}/{branch}", self.remote.name),
        }
    }

    /// The refspec fetched from the remote: the head branch into its tracking ref.
    pub fn fetch_refspec(&self) -> String {
        match &self.base.reference {
            BaseRef::HeadBranch(branch) => format!("+refs/heads/{branch}:{}", self.tracking_ref()),
        }
    }
}
