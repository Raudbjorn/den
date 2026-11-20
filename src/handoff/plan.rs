//! Planning a launch (pure): `plan(Work Item, Decision, target choice, Workspace
//! view, Job records, prompts) -> LaunchPlan or Refusal`. No I/O; the
//! WorkspaceView (which carries the Job records) was gathered by `inspect`.

use super::{
    Base, BaseRef, BranchStart, CloneSource, InspectRequest, JobBranch, LaunchPlan, MainClone,
    PathState, Refusal, RefusalReason, Remote, Target, TargetChoice, WindowMode, WorkspaceView,
    Writability,
    clone::{self, Candidate},
    command,
    inspect::CloneSearch,
    names,
    prompts::{PromptFields, Prompts, PromptsError},
    worktree_path,
};
use crate::{
    config::Config,
    decide::Decision,
    domain::{NextStep, Observation, Repo, WorkItemId},
};
use std::{collections::BTreeMap, path::PathBuf};

/// Where to look for Main Clones: the config's settings, with den's default root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchSettings {
    /// `main_clone_root` from the config, else den's default (`~/dev`).
    pub search_root: Option<PathBuf>,
    /// `[main_clones]`, keyed by `owner/name`.
    pub main_clones: BTreeMap<String, PathBuf>,
}

impl LaunchSettings {
    pub fn from_config(config: Option<&Config>, default_root: Option<PathBuf>) -> Self {
        Self {
            search_root: config
                .and_then(|c| c.main_clone_root.clone())
                .or(default_root),
            main_clones: config.map(|c| c.main_clones.clone()).unwrap_or_default(),
        }
    }

    /// The configured Main Clone for `repo` (`owner/name`, case-insensitive).
    fn configured_clone(&self, repo: &str) -> Option<PathBuf> {
        self.main_clones
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(repo))
            .map(|(_, path)| path.clone())
    }
}

/// Everything `plan` reads.
pub struct PlanInput<'a> {
    pub decision: &'a Decision,
    pub observation: &'a Observation,
    pub choice: TargetChoice,
    pub view: &'a WorkspaceView,
    pub prompts: Result<&'a Prompts, &'a PromptsError>,
}

/// What a launch needs from GitHub's facts alone.
struct Facts {
    step: NextStep,
    target: Target,
    number: u64,
    head_branch: String,
    head_commit: String,
}

/// The facts for a launch, or every reason it can't happen.
fn facts(
    decision: &Decision,
    observation: &Observation,
    _choice: TargetChoice,
) -> Result<Facts, Refusal> {
    let mut reasons = Vec::new();
    let step = decision.next_step;
    if step.is_none() {
        reasons.push(RefusalReason::NoNextStep);
    }
    let WorkItemId::PullRequest { repo, number } = &decision.item.id else {
        reasons.push(RefusalReason::NotYetLaunchable {
            kind: decision.item.id.kind(),
        });
        return Err(Refusal { reasons });
    };
    let Some(pr) = observation
        .pull_requests
        .iter()
        .find(|p| &p.repo == repo && p.number == *number)
    else {
        reasons.push(RefusalReason::WorkItemGone);
        return Err(Refusal { reasons });
    };
    let Some(repo_facts) = observation.repo(repo) else {
        reasons.push(RefusalReason::RepoUnknown { repo: repo.clone() });
        return Err(Refusal { reasons });
    };
    if repo_facts.is_archived {
        reasons.push(RefusalReason::RepoArchived { repo: repo.clone() });
    }
    if pr.head_repo.as_ref() != Some(repo) {
        reasons.push(RefusalReason::CrossRepoHead {
            head_repo: pr.head_repo.clone(),
        });
    }
    match step {
        Some(step) if reasons.is_empty() => Ok(Facts {
            step,
            target: target(repo_facts),
            number: *number,
            head_branch: pr.head_ref.clone(),
            head_commit: pr.head_commit.clone(),
        }),
        _ => Err(Refusal { reasons }),
    }
}

fn target(repo: &Repo) -> Target {
    Target {
        repo: repo.id.clone(),
        url: repo.url.clone(),
        is_fork: repo.is_fork,
    }
}

/// What to inspect for launching `decision`'s Work Item, or why den won't even
/// look (no Next Step, not a PR, archived, …).
pub fn inspect_request(
    decision: &Decision,
    observation: &Observation,
    choice: TargetChoice,
    settings: &LaunchSettings,
) -> Result<InspectRequest, Refusal> {
    let facts = facts(decision, observation, choice)?;
    let configured_clone = settings.configured_clone(&facts.target.repo.to_string());
    let search_root = match (&configured_clone, &settings.search_root) {
        (_, Some(root)) => root.clone(),
        // A configured clone needs no search.
        (Some(_), None) => PathBuf::new(),
        (None, None) => return Err(Refusal::one(RefusalReason::NoSearchRoot)),
    };
    Ok(InspectRequest {
        target: facts.target.repo.clone(),
        target_url: facts.target.url.clone(),
        configured_clone,
        search_root,
        worktree_name: names::worktree_name(&facts.head_branch),
        branch: facts.head_branch,
    })
}

/// Plans the launch, or refuses with every reason found.
pub fn plan(input: &PlanInput<'_>) -> Result<LaunchPlan, Refusal> {
    let facts = facts(input.decision, input.observation, input.choice)?;
    let view = input.view;
    let mut reasons = Vec::new();

    let prompts = match input.prompts {
        Ok(prompts) => Some(prompts),
        Err(error) => {
            reasons.push(RefusalReason::PromptsUnusable {
                error: error.to_string(),
            });
            None
        }
    };
    if let Err(error) = &view.jobs {
        reasons.push(RefusalReason::JobRecordsUnreadable {
            error: error.clone(),
        });
    }
    if view.window == WindowMode::Kitty && !view.claude_available {
        reasons.push(RefusalReason::ClaudeMissing);
    }

    let found = main_clone(&facts, view)
        .map_err(|reason| reasons.push(reason))
        .ok();
    let state = found.as_ref().and_then(|(clone, _)| {
        let chosen = view.chosen.as_ref().filter(|c| c.path == clone.path);
        if chosen.is_none() {
            reasons.push(RefusalReason::InspectionFailed {
                error: format!("{} was not inspected", clone.path.display()),
            });
        }
        chosen
    });

    let worktree_name = names::worktree_name(&facts.head_branch);
    let mut branch_start = None;
    let mut writable = Writability::Unknown("not inspected".to_owned());
    if let (Some((clone, remote)), Some(state)) = (&found, state) {
        match &state.state {
            PathState::Exists { writable: w } => {
                if *w == Writability::NotWritable {
                    reasons.push(RefusalReason::MainCloneNotWritable {
                        path: clone.path.clone(),
                    });
                }
                writable = w.clone();
            }
            PathState::Missing => reasons.push(RefusalReason::MainCloneMissing {
                path: clone.path.clone(),
            }),
            PathState::Unknown(error) => reasons.push(RefusalReason::MainCloneUnknown {
                path: clone.path.clone(),
                error: error.clone(),
            }),
        }
        let worktree = worktree_path(&clone.path, &worktree_name);
        let registered = match &state.worktrees {
            Ok(registered) => registered.as_slice(),
            Err(error) => {
                reasons.push(RefusalReason::InspectionFailed {
                    error: error.clone(),
                });
                &[]
            }
        };
        match &state.worktree {
            // Gone from disk but still registered: git would refuse the path.
            PathState::Missing if registered.iter().any(|w| w.path == worktree) => {
                reasons.push(RefusalReason::WorktreeRegistered {
                    path: worktree.clone(),
                });
            }
            PathState::Missing => {}
            PathState::Exists { .. } => reasons.push(RefusalReason::WorktreeExists {
                job: recorded_job(view, &input.decision.item.id, &worktree),
                path: worktree,
            }),
            PathState::Unknown(error) => reasons.push(RefusalReason::WorktreeUnknown {
                path: worktree,
                error: error.clone(),
            }),
        }
        match &state.local_branch {
            Ok(None) => {
                branch_start = Some(BranchStart::New {
                    tracking: format!("{}/{}", remote.name, facts.head_branch),
                });
            }
            Ok(Some(commit)) if *commit == facts.head_commit => {
                match registered
                    .iter()
                    .find(|w| w.branch.as_deref() == Some(facts.head_branch.as_str()))
                {
                    Some(elsewhere) => reasons.push(RefusalReason::BranchCheckedOut {
                        branch: facts.head_branch.clone(),
                        worktree: elsewhere.path.clone(),
                    }),
                    None => branch_start = Some(BranchStart::Existing),
                }
            }
            Ok(Some(commit)) => reasons.push(RefusalReason::LocalBranchDiffers {
                branch: facts.head_branch.clone(),
                commit: commit.clone(),
                base: facts.head_commit.clone(),
            }),
            Err(error) => reasons.push(RefusalReason::InspectionFailed {
                error: error.clone(),
            }),
        }
    }

    let (Some((mut clone, remote)), Some(prompts), Some(start), true) =
        (found, prompts, branch_start, reasons.is_empty())
    else {
        return Err(Refusal { reasons });
    };
    clone.writable = writable;
    let worktree = worktree_path(&clone.path, &worktree_name);
    let decision = input.decision;
    let prompt = prompts.render(
        facts.step,
        &PromptFields {
            repo: facts.target.repo.to_string(),
            number: Some(facts.number),
            url: decision.item.url.clone(),
            title: decision.item.title.clone(),
            branch: facts.head_branch.clone(),
        },
    );
    let session = names::pr_session_name(&facts.target.repo, facts.number, facts.step);
    let mut plan = LaunchPlan {
        work_item: decision.item.id.clone(),
        title: decision.item.title.clone(),
        url: decision.item.url.clone(),
        next_step: facts.step,
        target: facts.target,
        main_clone: clone,
        remote,
        base: Base {
            reference: BaseRef::HeadBranch(facts.head_branch.clone()),
            commit: facts.head_commit,
        },
        branch: JobBranch {
            name: facts.head_branch,
            start,
        },
        worktree,
        session,
        prompt,
        window: view.window.clone(),
        command: String::new(),
    };
    plan.command = command::display(&plan.window, &plan.window_command());
    Ok(plan)
}

/// The Main Clone and its remote for the target, or why there is none.
fn main_clone(facts: &Facts, view: &WorkspaceView) -> Result<(MainClone, Remote), RefusalReason> {
    let target = &facts.target;
    let unknown = Writability::Unknown("not inspected".to_owned());
    match &view.clones {
        CloneSearch::Configured { path, remotes } => {
            let remotes =
                remotes
                    .as_ref()
                    .map_err(|error| RefusalReason::ConfiguredCloneUnreadable {
                        path: path.clone(),
                        error: error.clone(),
                    })?;
            let remote = clone::remote_for(remotes, &target.url).ok_or_else(|| {
                RefusalReason::NoRemoteForTarget {
                    clone: path.clone(),
                    target: target.repo.clone(),
                    remotes: remotes.clone(),
                }
            })?;
            Ok((
                MainClone {
                    path: path.clone(),
                    source: CloneSource::Configured,
                    writable: unknown,
                },
                remote.clone(),
            ))
        }
        CloneSearch::Searched { root, candidates } => {
            match clone::choose(candidates, &target.url, &target.repo.name) {
                Some(choice) => Ok((
                    MainClone {
                        path: choice.path,
                        source: CloneSource::Searched {
                            root: root.clone(),
                            others: choice.others,
                        },
                        writable: unknown,
                    },
                    choice.remote,
                )),
                None => Err(RefusalReason::NoMainClone {
                    repo: target.repo.clone(),
                    root: root.clone(),
                    search_error: None,
                    near_misses: near_misses(candidates, facts),
                }),
            }
        }
        CloneSearch::SearchFailed { root, error } => Err(RefusalReason::NoMainClone {
            repo: target.repo.clone(),
            root: root.clone(),
            search_error: Some(error.clone()),
            near_misses: Vec::new(),
        }),
    }
}

fn near_misses(candidates: &[Candidate], facts: &Facts) -> Vec<(PathBuf, Vec<Remote>)> {
    clone::near_misses(candidates, &facts.target.url, &facts.target.repo.name)
        .map(|c| (c.path.clone(), c.remotes.clone().unwrap_or_default()))
        .collect()
}

/// The Job den recorded for this Work Item at `worktree`, if any.
fn recorded_job(
    view: &WorkspaceView,
    item: &WorkItemId,
    worktree: &std::path::Path,
) -> Option<(String, jiff::Timestamp)> {
    view.jobs
        .as_ref()
        .ok()?
        .iter()
        .rev()
        .find(|job| &job.work_item == item && job.worktree == worktree)
        .map(|job| (job.session.clone(), job.launched_at))
}
