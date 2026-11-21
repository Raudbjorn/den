//! Live Jobs (#14): recognising the Jobs den launched before.
//!
//! For each recorded Job den looks at its worktree and calls it
//! - **present**: the folder exists and git has it registered (a Live Job);
//! - **missing**: the folder is gone and git has nothing registered there;
//! - **unknown**: den couldn't tell (the path is unreadable, git's registration
//!   disagrees with the disk, the worktree is locked or prunable, or git
//!   couldn't list the worktrees). Unknown is never treated as gone.
//!
//! [`classify`] is pure. [`check_jobs`] gathers what it needs through the
//! Workspace port: one `git worktree list` per Main Clone (each with the
//! adapter's timeout), at most [`MAX_JOBS_CHECKED`] Jobs, within
//! [`JOB_CHECK_BUDGET_SECS`]. The App runs it as an Effect, off the UI thread.
//!
//! For each Live Job the same check says whether its Claude Code session is
//! **running** or **idle** (#80, spec 0004 stories 67–69): running when a
//! local `claude` process of the user has `-n <session>` or `--resume
//! <session>` in its arguments ([`running_sessions`]), read once per check
//! through [`Workspace::user_processes`]. An unreadable process list is
//! unknown. It is display only: the worktree alone decides whether a Job is
//! Live ([`JobCheck::is_live`]) and so whether a launch asks first.
//!
//! A Live Job can be reopened ([`plan_reopen`], [`reopen`]): a new window in
//! its worktree resuming its recorded session. Never in the Main Clone itself,
//! and nothing is ever deleted.

use super::{
    PathState, ProjectSettings, Refusal, RefusalReason, RegisteredWorktree, WORKTREES_DIR,
    WindowMode,
    command::{self, PermissionMode, SessionStart, WindowCommand},
    execute::{LaunchOutcome, LaunchReport, LaunchStep},
    inspect::WorkspaceView,
    names,
};
use crate::{
    domain::{LinearWorkflowPolicy, WorkItemId},
    jobs::JobRecord,
    ports::{Launcher, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// The most Jobs one check looks at (the most recently launched ones).
pub const MAX_JOBS_CHECKED: usize = 200;
/// The most one check of Job worktrees may take, in seconds; Jobs left when it
/// runs out are reported unknown, not missing.
pub const JOB_CHECK_BUDGET_SECS: u64 = 20;

/// What a recorded Job's worktree looks like now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeState {
    /// It exists and git has it registered: the Job is a Live Job.
    Present,
    /// Its folder is gone and git has nothing registered there.
    Missing,
    /// den couldn't tell, and why. Never treated as gone.
    Unknown(String),
}

/// Whether a Live Job's Claude Code session has a local process (#80).
/// Display only: never decides whether a Job is Live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionActivity {
    /// A local `claude` process of the user names the Job's session.
    Running,
    /// No local `claude` process of the user names it: the session ended,
    /// or crashed.
    Idle,
    /// den couldn't read the process list, and why.
    Unknown(String),
}

/// One recorded Job and its worktree's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobCheck {
    pub job: JobRecord,
    pub state: WorktreeState,
    /// Whether its session is running; `None` unless the Job is Live.
    pub activity: Option<SessionActivity>,
}

impl JobCheck {
    pub fn is_live(&self) -> bool {
        self.state == WorktreeState::Present
    }
}

/// The result of checking every recorded Job.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobsCheck {
    /// Newest first, one per Work Item and worktree.
    pub checks: Vec<JobCheck>,
    /// Jobs beyond [`MAX_JOBS_CHECKED`] that were not looked at.
    pub not_checked: usize,
}

/// The folder Jobs' worktrees live in, inside `main_clone`.
fn worktrees_dir(main_clone: &Path) -> PathBuf {
    WORKTREES_DIR
        .iter()
        .fold(main_clone.to_path_buf(), |path, part| path.join(part))
}

/// The recorded worktree is a folder directly inside the Main Clone's
/// `.claude/worktrees/`, where den puts Jobs, and so never the Main Clone itself.
pub fn is_job_worktree_path(job: &JobRecord) -> bool {
    job.worktree.parent() == Some(worktrees_dir(&job.main_clone).as_path())
        && job.worktree.file_name().is_some()
}

/// The state of `job`'s worktree, from what is at its path and what git has
/// registered for its Main Clone (pure).
pub fn classify(
    job: &JobRecord,
    path: &PathState,
    registered: &Result<Vec<RegisteredWorktree>, String>,
) -> WorktreeState {
    let unknown = |why: String| WorktreeState::Unknown(why);
    if !is_job_worktree_path(job) {
        return unknown(format!(
            "the recorded worktree {} isn't inside {}, where den puts Jobs",
            job.worktree.display(),
            worktrees_dir(&job.main_clone).display()
        ));
    }
    let listed = match (path, registered) {
        (PathState::Unknown(error), _) => {
            return unknown(format!("den can't inspect the folder: {error}"));
        }
        // Nothing on disk: whatever git says, there is nothing to reopen or lose.
        (PathState::Missing, Err(_)) => return WorktreeState::Missing,
        (PathState::Exists { .. }, Err(error)) => {
            return unknown(format!(
                "git couldn't list the Main Clone's worktrees: {error}"
            ));
        }
        (_, Ok(listed)) => listed,
    };
    let entry = listed.iter().position(|w| w.path == job.worktree);
    match (path, entry.map(|i| (i, &listed[i]))) {
        // git lists the clone's own checkout first.
        (_, Some((0, _))) => unknown(
            "git lists that path as the Main Clone's own checkout, not a Job's worktree".to_owned(),
        ),
        (_, Some((_, w))) if w.locked => unknown("git has the worktree locked".to_owned()),
        (_, Some((_, w))) if w.prunable => unknown(
            "git has the worktree registered but marks it prunable (its folder is gone or unreadable to git)"
                .to_owned(),
        ),
        (PathState::Exists { .. }, Some(_)) => WorktreeState::Present,
        (PathState::Exists { .. }, None) => {
            unknown("the folder exists but git has no worktree registered there".to_owned())
        }
        (PathState::Missing, Some(_)) => {
            unknown("git has a worktree registered there but its folder is gone".to_owned())
        }
        (PathState::Missing, None) => WorktreeState::Missing,
        (PathState::Unknown(_), _) => unknown("den can't inspect the folder".to_owned()),
    }
}

/// The newest record per Work Item and worktree, newest first.
fn latest(jobs: &[JobRecord]) -> Vec<&JobRecord> {
    let mut sorted: Vec<&JobRecord> = jobs.iter().collect();
    // Stable: among equal times the later record (appended last) wins.
    sorted.reverse();
    sorted.sort_by_key(|job| std::cmp::Reverse(job.launched_at));
    let mut seen = BTreeSet::new();
    sorted
        .into_iter()
        .filter(|job| seen.insert((job.subject.folded(), job.worktree.clone())))
        .collect()
}

/// Claude Code's flags that name a session in a command line: den starts
/// sessions with `-n <session>` and reopens them with `--resume <session>`
/// ([`command`]).
const SESSION_FLAGS: [&str; 2] = ["-n", "--resume"];

/// The sessions named by the `claude` processes in `processes` (each a
/// command line, program first): the word after `-n` or `--resume` in a
/// process whose program is `claude` (pure). Other programs called `claude`
/// (Claude Desktop's) never pass those flags, so they name nothing.
pub fn running_sessions(processes: &[Vec<String>]) -> BTreeSet<String> {
    processes
        .iter()
        .filter(|argv| {
            argv.first()
                .and_then(|program| Path::new(program).file_name())
                .is_some_and(|name| name == command::CLAUDE_PROGRAM)
        })
        .flat_map(|argv| {
            argv.windows(2)
                .skip(1)
                .filter(|pair| SESSION_FLAGS.contains(&pair[0].as_str()))
                .map(|pair| pair[1].clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `session`'s activity, given the sessions read from the process list, or
/// why it couldn't be read (pure).
pub fn activity(session: &str, sessions: &Result<BTreeSet<String>, String>) -> SessionActivity {
    match sessions {
        Ok(sessions) if sessions.contains(session) => SessionActivity::Running,
        Ok(_) => SessionActivity::Idle,
        Err(why) => SessionActivity::Unknown(why.clone()),
    }
}

/// Checks the worktrees of `jobs` (the newest [`MAX_JOBS_CHECKED`] of them)
/// through `workspace`, listing each Main Clone's worktrees once. Jobs left
/// when `budget` runs out are unknown. When any Job is Live, the user's
/// processes are read once to tell each Live Job running or idle.
pub fn check_jobs(jobs: &[JobRecord], workspace: &dyn Workspace, budget: Duration) -> JobsCheck {
    let latest = latest(jobs);
    let not_checked = latest.len().saturating_sub(MAX_JOBS_CHECKED);
    let started = Instant::now();
    let mut listings: BTreeMap<PathBuf, Result<Vec<RegisteredWorktree>, String>> = BTreeMap::new();
    let checks = latest
        .into_iter()
        .take(MAX_JOBS_CHECKED)
        .map(|job| {
            let state = if started.elapsed() > budget {
                WorktreeState::Unknown(format!(
                    "not checked: den's check of Job worktrees ran out of its {}s",
                    budget.as_secs()
                ))
            } else {
                let path = workspace.path_state(&job.worktree);
                let registered = listings.entry(job.main_clone.clone()).or_insert_with(|| {
                    workspace
                        .worktrees(&job.main_clone)
                        .map_err(|e| e.to_string())
                });
                classify(job, &path, registered)
            };
            JobCheck {
                job: job.clone(),
                state,
                activity: None,
            }
        })
        .collect::<Vec<_>>();
    let checks = with_activity(checks, workspace, started, budget);
    JobsCheck {
        checks,
        not_checked,
    }
}

/// `checks` with each Live Job's [`SessionActivity`], from one read of the
/// user's processes; none is read when no Job is Live. Unknown when the
/// budget already ran out.
fn with_activity(
    mut checks: Vec<JobCheck>,
    workspace: &dyn Workspace,
    started: Instant,
    budget: Duration,
) -> Vec<JobCheck> {
    if !checks.iter().any(JobCheck::is_live) {
        return checks;
    }
    let sessions = if started.elapsed() > budget {
        Err(format!(
            "not checked: den's check of Jobs ran out of its {}s",
            budget.as_secs()
        ))
    } else {
        workspace
            .user_processes()
            .map(|processes| running_sessions(&processes))
            .map_err(|e| format!("den couldn't read the local processes: {}", e.detail))
    };
    for check in checks.iter_mut().filter(|c| c.is_live()) {
        check.activity = Some(activity(&check.job.session, &sessions));
    }
    checks
}

/// The check that says most about `item`'s Jobs: its newest Live Job, else
/// its newest unknown one, else its newest missing one; `None` without Jobs.
pub fn most_telling<'a>(checks: &'a [JobCheck], item: &WorkItemId) -> Option<&'a JobCheck> {
    let mine = || checks.iter().filter(|c| c.job.subject.covers(item));
    mine()
        .find(|c| c.is_live())
        .or_else(|| mine().find(|c| matches!(c.state, WorktreeState::Unknown(_))))
        .or_else(|| mine().next())
}

/// The smallest suffix `n` for which `<worktree_name>-n` and `<branch>-n` are
/// used by no recorded Job and no registered worktree in `view`; `None` when
/// none up to [`names::MAX_NAME_SUFFIX`] is free. The inspection that follows still
/// checks the path is free and the branch doesn't exist.
pub fn fresh_suffix(view: &WorkspaceView, worktree_name: &str, branch: &str) -> Option<u32> {
    let mut names_taken = BTreeSet::new();
    let mut branches_taken = BTreeSet::new();
    let file_name = |path: &Path| path.file_name().map(|n| n.to_string_lossy().into_owned());
    for job in view.jobs.iter().flatten() {
        names_taken.extend(file_name(&job.worktree));
        branches_taken.insert(job.branch.clone());
    }
    let registered = view.chosen.as_ref().and_then(|c| c.worktrees.as_ref().ok());
    for worktree in registered.into_iter().flatten() {
        names_taken.extend(file_name(&worktree.path));
        branches_taken.extend(worktree.branch.clone());
    }
    (names::FIRST_NAME_SUFFIX..=names::MAX_NAME_SUFFIX).find(|&n| {
        !names_taken.contains(&names::suffixed(
            worktree_name,
            n,
            names::MAX_WORKTREE_NAME_CHARS,
        )) && !branches_taken.contains(&names::suffixed(branch, n, names::MAX_BRANCH_NAME_CHARS))
    })
}

/// Opening an existing Live Job: a new window in its worktree resuming its
/// recorded session. Nothing is fetched, added or recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReopenPlan {
    pub job: JobRecord,
    /// Decided again on reopening, from the survey den shows then
    /// ([`super::trust::for_reopen`]): a resumed session runs hooks too.
    pub project_settings: ProjectSettings,
    /// The mode the session is resumed in ([`JobRecord::reopen_mode`] for the
    /// Job's Owner's Linear Workflow policy).
    pub permission_mode: Option<PermissionMode>,
    pub window: WindowMode,
    /// Environment variables the resumed session runs without (#70), as a
    /// new Job's.
    pub removed_env: Vec<String>,
    /// The exact command, as shown on the sheet.
    pub command: String,
}

impl ReopenPlan {
    /// What the window runs: `claude --resume <session>` in the Job's
    /// worktree, in the Job's permission mode ([`Self::permission_mode`]).
    /// Claude Code would otherwise restore the mode the session ended in, or a
    /// settings file's `defaultMode`, so a coordination Job passes its mode
    /// again. Its Ground Rules are passed again too, rebuilt from the Job's
    /// record as at launch (ADR-0008).
    pub fn window_command(&self) -> WindowCommand {
        WindowCommand {
            directory: self.job.worktree.clone(),
            session: self.job.session.clone(),
            prompt: String::new(),
            start: SessionStart::Resume,
            permission_mode: self.permission_mode,
            setting_sources: self.project_settings.sources(),
            removed_env: self.removed_env.clone(),
            ground_rules: super::GroundRules::for_job(self.job.next_step, &self.job.subject).text(),
        }
    }
}

/// A session name den would have made: safe as one argument, not option-like.
fn session_usable(session: &str) -> bool {
    !session.is_empty()
        && !session.starts_with(['-', '.'])
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Plans reopening `check`'s Job, or refuses: it must be a Live Job, its
/// worktree must be inside the Main Clone's `.claude/worktrees/` (never the
/// Main Clone itself), and its session name must be one den would make.
/// `policy` is the Job's Owner's Linear Workflow policy (the empty one
/// without a workflow): a record without a mode reopens in the mode its
/// Next Step launches in for that Owner.
/// `project_settings` is [`super::trust::for_reopen`]'s for the Job.
pub fn plan_reopen(
    check: &JobCheck,
    view: &WorkspaceView,
    policy: &LinearWorkflowPolicy,
    project_settings: ProjectSettings,
    removed_env: &[String],
) -> Result<ReopenPlan, Refusal> {
    let job = &check.job;
    let mut reasons = Vec::new();
    if !is_job_worktree_path(job) {
        reasons.push(RefusalReason::ReopenOutsideWorktrees {
            worktree: job.worktree.clone(),
            main_clone: job.main_clone.clone(),
        });
    }
    match &check.state {
        WorktreeState::Present => {}
        WorktreeState::Missing => reasons.push(RefusalReason::JobNotLive {
            worktree: job.worktree.clone(),
            why: "its folder is gone".to_owned(),
        }),
        WorktreeState::Unknown(why) => reasons.push(RefusalReason::JobNotLive {
            worktree: job.worktree.clone(),
            why: why.clone(),
        }),
    }
    if !session_usable(&job.session) {
        reasons.push(RefusalReason::SessionNameUnusable {
            session: job.session.clone(),
        });
    }
    if view.window == WindowMode::Kitty && !view.claude_available {
        reasons.push(RefusalReason::ClaudeMissing);
    }
    if !reasons.is_empty() {
        return Err(Refusal { reasons });
    }
    let mut plan = ReopenPlan {
        job: job.clone(),
        project_settings,
        permission_mode: job.reopen_mode(policy),
        window: view.window.clone(),
        removed_env: removed_env.to_vec(),
        command: String::new(),
    };
    plan.command = command::display(&plan.window, &plan.window_command());
    Ok(plan)
}

/// Carries `plan` out: re-checks the worktree is still a Live Job's, then
/// opens the window (or copies the command). Records nothing, deletes nothing.
pub fn reopen(
    plan: &ReopenPlan,
    workspace: &dyn Workspace,
    launcher: &dyn Launcher,
) -> LaunchReport {
    let mut done = Vec::new();
    let job = &plan.job;
    let registered = workspace
        .worktrees(&job.main_clone)
        .map_err(|e| e.to_string());
    let state = classify(job, &workspace.path_state(&job.worktree), &registered);
    let outcome = match state {
        WorktreeState::Present => {
            done.push(format!(
                "re-verified: {} still exists and is registered with git",
                job.worktree.display()
            ));
            open(plan, launcher, &mut done)
        }
        WorktreeState::Missing => LaunchOutcome::Refused(Refusal::one(RefusalReason::JobNotLive {
            worktree: job.worktree.clone(),
            why: "its folder is gone".to_owned(),
        })),
        WorktreeState::Unknown(why) => {
            LaunchOutcome::Refused(Refusal::one(RefusalReason::JobNotLive {
                worktree: job.worktree.clone(),
                why,
            }))
        }
    };
    LaunchReport { done, outcome }
}

fn open(plan: &ReopenPlan, launcher: &dyn Launcher, done: &mut Vec<String>) -> LaunchOutcome {
    let window = plan.window_command();
    match &plan.window {
        WindowMode::Kitty => {
            if let Err(e) = launcher.open_window(&window) {
                return LaunchOutcome::Failed {
                    step: LaunchStep::OpenWindow,
                    error: e.to_string(),
                };
            }
            done.push(format!(
                "opened a kitty window resuming session {}",
                plan.job.session
            ));
        }
        WindowMode::Clipboard { .. } => {
            if let Err(e) = launcher.copy_to_clipboard(&command::clipboard_text(&window)) {
                return LaunchOutcome::Failed {
                    step: LaunchStep::CopyCommand,
                    error: e.to_string(),
                };
            }
            done.push("copied the command to the clipboard".to_owned());
        }
    }
    LaunchOutcome::Reopened {
        job: plan.job.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionActivity, activity, running_sessions};
    use std::collections::BTreeSet;

    fn argvs(processes: &[&[&str]]) -> Vec<Vec<String>> {
        processes
            .iter()
            .map(|argv| argv.iter().map(|w| (*w).to_owned()).collect())
            .collect()
    }

    #[test]
    fn only_claude_processes_name_sessions_with_n_or_resume() {
        let sessions = running_sessions(&argvs(&[
            &[
                "/opt/claude-code/bin/claude",
                "-n",
                "a-pr1-fix-ci",
                "prompt",
            ],
            &[
                "claude",
                "--permission-mode",
                "manual",
                "--resume",
                "a-pr2-land",
            ],
            // Not `claude`: the program's file name decides, not a substring.
            &["/usr/bin/claude-desktop", "-n", "not-1"],
            &["vim", "-n", "not-2"],
            &["sh", "-c", "claude -n not-3"],
            // A flag with no value names nothing; nor does the program word.
            &["claude", "-n"],
            &["-n", "not-4"],
            &[],
        ]));
        assert_eq!(
            sessions,
            BTreeSet::from(["a-pr1-fix-ci".to_owned(), "a-pr2-land".to_owned()])
        );
    }

    #[test]
    fn a_session_is_running_idle_or_unknown_from_the_read() {
        let read = Ok(BTreeSet::from(["s".to_owned()]));
        assert_eq!(activity("s", &read), SessionActivity::Running);
        assert_eq!(activity("t", &read), SessionActivity::Idle);
        assert_eq!(
            activity("s", &Err("no /proc".to_owned())),
            SessionActivity::Unknown("no /proc".to_owned())
        );
    }
}
