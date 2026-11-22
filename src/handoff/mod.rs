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
//! Every kind of Work Item launches (#12): a PR on its head branch; an Issue
//! or a default-branch Run on a new `<type>_<slug>` branch from the target's
//! default branch ([`BaseRef::DefaultBranch`], [`BranchStart::Fresh`]); a
//! Branch on itself. [`kinds`] holds the non-PR facts. When GitHub doesn't say
//! which commit the base is at, `inspect` asks the remote, so every sheet shows
//! a commit and the executor re-checks it like a PR head. A Repo with no Main
//! Clone is cloned ([`cloning`], [`CloneSource::ToClone`]) as the executor's
//! first change. A worktree name another Work Item's Job already has gets a
//! numeric suffix ([`names::free_worktree_name`], [`LaunchPlan::renamed`]).
//!
//! Extension points for later tickets:
//! - #13 (done): [`TargetChoice::Upstream`] targets a fork's Upstream (the
//!   fork's setting, or `u` on the sheet for one Job); [`Target::upstream_of`]
//!   says so and [`Target::default_branch`] is the branch new `<type>_<slug>`
//!   branches start from (the Upstream's when it is the target). A PR's head in
//!   another Repo is [`BaseRef::PullHead`], fetched as `refs/pull/<n>/head` from
//!   the target's remote. A PR is only ever fetched from the Repo it is in: an
//!   Upstream target for a fork's PR is [`RefusalReason::PullRequestNotInTarget`],
//!   and for a fork's Branch [`RefusalReason::BranchNotInTarget`].
//! - #14 (Live Jobs, built): [`live`] tells each recorded Job's worktree
//!   present / missing / unknown. The WorkspaceView carries this Work Item's
//!   checks (`job_checks`); the App asks "open existing or start fresh?" on a
//!   Live Job. Open existing is a [`live::ReopenPlan`] carried out by
//!   [`live::reopen`] (`claude --resume`, never in the Main Clone); start fresh
//!   is [`PlanInput::fresh`], a suffix on the worktree name, branch and
//!   session.
//!
//! One suffix rule for every name den suffixes ([`names::suffixed`],
//! `<name>-<n>`): a worktree name another Work Item's Job has (#12), and a
//! fresh start beside this Work Item's Live Job (#14).
//!
//! Linear Issues (#38): a Start or Implement Job goes to the Repo chosen on
//! the sheet from the Linear Workflow's Repos for new work
//! ([`PlanInput::linear_repo`]), on a new branch from its default branch
//! (Linear's suggested name when it is safe), with the `implement_linear`
//! prompt (identifier, URL, Repo). Its record's subject is
//! [`JobSubject::LinearIssue`](crate::jobs::JobSubject::LinearIssue). The
//! Linear writes before it are the App's (a re-read, then assign, then move
//! state), never the executor's.
//!
//! Coordination Jobs (#39): a Request review (and, with #40, a Land) Job acts
//! on GitHub rather than on code. It still gets a worktree on the PR head, as
//! a PR Job does, but runs in the approval-required permission mode
//! ([`job_permission_mode`], [`LaunchPlan::permission_mode`]), recorded with
//! the Job so reopening it asks again; a record without the mode falls back
//! to its Next Step's and Owner's ([`crate::jobs::JobRecord::reopen_mode`]).
//! Its prompt gets no push line.
//!
//! Review Jobs in a Linear Workflow (#41) run in the same mode, from the
//! Owner's policy rather than the Next Step alone. Their prompt is the
//! `review_linear` template ([`prompts::PromptKind`]): the verdict-only
//! instruction and, for a Pull Request in a Landing Group, the Linear
//! Issue's identifier and URL. Every sheet lists the slash commands its
//! prompt invokes as found or not found ([`workflows`]), from the Claude
//! command and skill directories `inspect` read. A Request review plan also carries the reviewers chosen
//! on the sheet ([`PlanInput::reviewers`], [`reviewers`]) and the Owner's
//! Slack channel, both only in its prompt.
//!
//! Land (#40): `c` on a Linear Issue whose Next Step is Land plans a
//! coordination Job for its Landing Group ([`landing`]): the open members
//! outside Held Repos in landing order, each pinned to its head, with a
//! worktree on the first member's head. The plan carries the group
//! ([`LaunchPlan::landing`]); the executor re-reads every member's head from
//! GitHub before it changes anything, and the Job record's subject is the
//! Landing Group ([`crate::jobs::JobSubject::LandingGroup`]), which covers
//! the Linear Issue and every landed member.

pub mod clone;
pub mod cloning;
pub mod command;
mod execute;
pub mod ground_rules;
mod inspect;
mod kinds;
pub mod landing;
pub mod live;
pub mod names;
mod plan;
pub mod prompts;
mod refusal;
pub mod reviewers;
pub mod trust;
pub mod workflows;

pub use clone::Remote;
pub use cloning::NewCloneView;
pub use command::{PermissionMode, SessionStart, SettingSources, WindowCommand, WindowMode};
pub use execute::{LaunchOutcome, LaunchReport, LaunchStep, execute};
pub use ground_rules::GroundRules;
pub use inspect::{ChosenClone, CloneSearch, InspectRequest, WorkspaceView, inspect};
pub use landing::{HeldMember, LandingGroupPlan, LandingMember};
pub use live::{JobCheck, JobsCheck, ReopenPlan, SessionActivity, WorktreeState};
pub use plan::{LaunchSettings, PlanInput, inspect_request, plan};
pub use refusal::{Refusal, RefusalReason};
pub use trust::{OffReason, ProjectSettings};

use crate::domain::{LinearWorkflowPolicy, NextStep, RepoId, WorkItemId, facts::Reviewer};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where Jobs' worktrees live inside a Main Clone (where Claude Code expects them).
pub const WORKTREES_DIR: [&str; 2] = [".claude", "worktrees"];
/// The longest prompt den passes to Claude Code, in characters.
pub const MAX_PROMPT_CHARS: usize = 8_000;
/// Characters of a commit shown on the sheet.
pub const SHORT_COMMIT_CHARS: usize = 7;

/// The permission mode a Job for `step` runs in, given its Owner's Linear
/// Workflow `policy` (the empty policy for an Owner without one):
/// approval-required for a coordination Job (Request review, Land, and Merge
/// since #72), Add screenshots (#79), and for a
/// Review Job in a Linear Workflow, which returns a verdict only (#41,
/// [`LinearWorkflowPolicy::reviews_verdict_only`]); none (milestone 1's
/// command) for every other Job. The match is exhaustive, so a new kind
/// must choose.
pub fn job_permission_mode(
    step: NextStep,
    policy: &LinearWorkflowPolicy,
) -> Option<PermissionMode> {
    match step {
        // Add screenshots uploads images and edits the PR body: every step
        // is seen before it happens (#79).
        NextStep::RequestReview | NextStep::Land | NextStep::Merge | NextStep::AddScreenshots => {
            Some(PermissionMode::ApprovalRequired)
        }
        NextStep::Review if policy.reviews_verdict_only() => Some(PermissionMode::ApprovalRequired),
        NextStep::Review
        | NextStep::FinishDraft
        | NextStep::ResolveConflicts
        | NextStep::FixCi
        | NextStep::AddressReview
        | NextStep::Triage
        | NextStep::Implement
        | NextStep::OpenPr
        | NextStep::PruneOrRevive
        | NextStep::Start
        | NextStep::MarkForRelease => None,
    }
}

/// Whether a Job for `step` coordinates rather than changes code: it runs
/// in the approval-required mode ([`job_permission_mode`]) and acts on GitHub,
/// not on its branch, so it gets no push line and never the repo's project
/// settings. Add screenshots runs in that mode too but is not one (#79,
/// CONTEXT.md): it may commit to its own PR's branch, which is the viewer's.
pub fn coordinates(step: NextStep, policy: &LinearWorkflowPolicy) -> bool {
    step != NextStep::AddScreenshots && job_permission_mode(step, policy).is_some()
}

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

/// Which Repo the Job's work goes to (ADR-0003: a fork is its own target
/// unless its Upstream is chosen, for the fork or for one Job).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetChoice {
    /// The Work Item's own Repo (for a fork: the fork itself).
    #[default]
    OwnRepo,
    /// The fork's Upstream. Only meaningful for a Work Item in a fork.
    Upstream,
}

impl TargetChoice {
    /// The other choice (`u`).
    pub fn flipped(self) -> Self {
        match self {
            Self::OwnRepo => Self::Upstream,
            Self::Upstream => Self::OwnRepo,
        }
    }

    /// The target a launch sheet for `item` starts from, given the fork's own
    /// setting (`fork_setting`). A PR's and a Branch's refs live in the Repo
    /// they are in, so their Jobs start on that Repo whatever the fork's
    /// setting (a sheet's `u` can still choose the Upstream, which is
    /// refused with the reason). Issues and Runs follow the fork's setting:
    /// their new branches start from the target's default branch.
    pub fn default_for(item: &WorkItemId, fork_setting: Self) -> Self {
        match item {
            WorkItemId::PullRequest { .. } | WorkItemId::Branch { .. } => Self::OwnRepo,
            // A Linear Issue's Job (#38) starts a new branch in the Repo
            // chosen for it, so it follows that Repo's fork setting.
            WorkItemId::Issue { .. } | WorkItemId::Run { .. } | WorkItemId::LinearIssue { .. } => {
                fork_setting
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub repo: RepoId,
    /// The Repo's URL on GitHub; a remote matches it when both name the same Repo.
    pub url: String,
    pub is_fork: bool,
    /// The fork whose Upstream this is, when the target is an Upstream.
    pub upstream_of: Option<RepoId>,
    /// The target's default branch (new branches start there, #12); `None`
    /// for an empty Repo.
    pub default_branch: Option<String>,
}

/// How the Main Clone was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloneSource {
    /// By matching remotes under `root`; `others` also matched and lost.
    Searched { root: PathBuf, others: Vec<PathBuf> },
    /// From `[main_clones]` in the config.
    Configured,
    /// None exists yet: launching clones `url` into the MainClone's path, a
    /// new folder under `root`. `taken` are the folders den passed over
    /// because something is already there.
    ToClone {
        url: String,
        root: PathBuf,
        taken: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainClone {
    pub path: PathBuf,
    pub source: CloneSource,
    pub writable: Writability,
}

/// What the Job's worktree starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseRef {
    /// A branch of the target, fetched by name: a same-Repo PR's head branch,
    /// or the Branch a Branch Work Item is.
    HeadBranch(String),
    /// The target's default branch, for a new `<type>_<slug>` branch (#12).
    DefaultBranch(String),
    /// The head of PR `number`, whose head branch lives in another Repo than
    /// the target (or in one that was deleted): fetched as
    /// `refs/pull/<number>/head` from the target's remote, never from the
    /// head's Repo (#13).
    PullHead { number: u64 },
}

impl BaseRef {
    /// The target's branch the base is fetched from; `None` for a PR ref.
    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::HeadBranch(branch) | Self::DefaultBranch(branch) => Some(branch),
            Self::PullHead { .. } => None,
        }
    }

    /// The ref on the target's remote: `refs/heads/<branch>` or
    /// `refs/pull/<n>/head`.
    pub fn remote_ref(&self) -> String {
        match self {
            Self::HeadBranch(branch) | Self::DefaultBranch(branch) => {
                format!("refs/heads/{branch}")
            }
            Self::PullHead { number } => format!("refs/pull/{number}/head"),
        }
    }
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
    /// Created at the base, tracking nothing: the head branch lives in another
    /// Repo, and the target's remote has no branch of that name to track.
    NewUntracked,
    /// Already exists locally at the base commit; checked out as is.
    Existing,
    /// A new branch for this Job, created at the base and tracking nothing:
    /// pushing it creates it on the remote.
    Fresh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobBranch {
    pub name: String,
    pub start: BranchStart,
    /// The name it would have had, when that clashed in the Main Clone: a
    /// cross-Repo PR head named like the default branch or a branch already
    /// there is checked out as `pr-<n>-<head>` instead.
    pub renamed_from: Option<String>,
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
    /// Set when the wanted worktree name was taken by another Work Item's Job.
    pub renamed: Option<Renamed>,
    pub session: String,
    pub prompt: String,
    /// The approval-required mode for a coordination Job and for a Review Job
    /// in a Linear Workflow; `None` for any other Job, whose command is
    /// milestone 1's ([`job_permission_mode`]).
    pub permission_mode: Option<PermissionMode>,
    /// Off (`--setting-sources user`) for a coordination Job and for a PR
    /// head the viewer didn't write ([`trust`]).
    pub project_settings: ProjectSettings,
    /// Request review only: whom the prompt asks, and where it announces.
    pub review_request: Option<ReviewRequest>,
    /// Land only (#40): the members the Job lands, in order, pinned to their
    /// heads, and the held members it leaves alone.
    pub landing: Option<LandingGroupPlan>,
    pub window: WindowMode,
    /// Environment variables the Job runs without (#70, ADR-0007).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_env: Vec<String>,
    /// The exact command, as shown on the sheet.
    pub command: String,
}

/// What a Request review Job's prompt names (#39), besides the PR's URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequest {
    /// The reviewers to request on GitHub, as chosen on the sheet.
    pub reviewers: Vec<Reviewer>,
    /// The Owner's Slack channel, opaque; `None`: no announcement step.
    pub slack_channel: Option<String>,
    /// The prompt carries `{announcement}`: the announcement step with a
    /// channel, the outcome report without. A custom `request_review`
    /// template may leave it out; then the prompt neither announces nor asks
    /// for the outcome, whatever `slack_channel` says.
    pub announces: bool,
}

/// A worktree name (and, for a new branch, the branch name) that got a
/// numeric suffix because other Work Items' Jobs already have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renamed {
    /// The name den wanted.
    pub wanted: String,
    /// The subjects of the Jobs that hold that name (or an earlier suffix).
    pub taken_by: Vec<crate::jobs::JobSubject>,
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
    /// What the Job's record is for: the Landing Group for a Land Job, the
    /// Work Item otherwise.
    pub fn subject(&self) -> crate::jobs::JobSubject {
        match &self.landing {
            Some(group) => crate::jobs::JobSubject::LandingGroup(group.subject()),
            None => crate::jobs::JobSubject::of(&self.work_item),
        }
    }

    /// What the Job's window runs.
    pub fn window_command(&self) -> WindowCommand {
        WindowCommand {
            directory: self.worktree.clone(),
            session: self.session.clone(),
            prompt: self.prompt.clone(),
            start: SessionStart::New,
            permission_mode: self.permission_mode,
            setting_sources: self.project_settings.sources(),
            removed_env: self.removed_env.clone(),
            ground_rules: GroundRules::for_job(self.next_step, &self.subject()).text(),
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

    /// The plan on `commit` instead (the base moved and the user accepted it).
    pub fn on_commit(mut self, commit: &str) -> Self {
        commit.clone_into(&mut self.base.commit);
        self
    }

    /// The ref the base is fetched into: `refs/remotes/<remote>/<branch>` for
    /// a branch, `refs/den/pull/<remote>/<n>` for a PR ref. A PR head is kept
    /// out of `refs/remotes/`: the forced fetch would otherwise overwrite the
    /// remote-tracking ref of a real branch named `pr/<n>`.
    pub fn tracking_ref(&self) -> String {
        match &self.base.reference {
            BaseRef::HeadBranch(branch) | BaseRef::DefaultBranch(branch) => {
                format!("refs/remotes/{}/{branch}", self.remote.name)
            }
            BaseRef::PullHead { number } => {
                format!("refs/den/pull/{}/{number}", self.remote.name)
            }
        }
    }

    /// The refspec fetched from the remote: the base branch, or the PR ref,
    /// into [`Self::tracking_ref`].
    pub fn fetch_refspec(&self) -> String {
        format!(
            "+{}:{}",
            self.base.reference.remote_ref(),
            self.tracking_ref()
        )
    }

    /// The Main Clone doesn't exist yet: launching clones it first.
    pub fn clones(&self) -> bool {
        matches!(self.main_clone.source, CloneSource::ToClone { .. })
    }
}
