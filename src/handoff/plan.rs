//! Planning a launch (pure): `plan(Work Item, Decision, target choice, Workspace
//! view, Job records, prompts) -> LaunchPlan or Refusal`. No I/O; the
//! WorkspaceView (which carries the Job records) was gathered by `inspect`.

use super::{
    Base, BaseRef, BranchStart, ChosenClone, CloneSource, InspectRequest, JobBranch, LaunchPlan,
    MainClone, NewCloneView, PathState, Refusal, RefusalReason, RegisteredWorktree, Remote,
    Renamed, Target, TargetChoice, WindowMode, WorkspaceView, Writability,
    clone::{self, Candidate},
    cloning, command,
    inspect::CloneSearch,
    kinds::{self, Facts},
    live::MAX_FRESH_BRANCH_CHARS,
    names,
    prompts::{PromptFields, Prompts, PromptsError},
    worktree_path,
};
use crate::{
    config::Config,
    decide::Decision,
    domain::{Observation, WorkItemId},
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
    /// Start fresh beside a Live Job (#14): the suffix for the worktree name,
    /// the local branch and the session name. `None` for a plain launch.
    pub fresh: Option<u32>,
}

/// The facts for a launch, or every reason it can't happen. PRs here; Issues,
/// Branches and Runs in [`kinds`].
fn facts(
    decision: &Decision,
    observation: &Observation,
    choice: TargetChoice,
) -> Result<Facts, Refusal> {
    let WorkItemId::PullRequest { repo, number } = &decision.item.id else {
        return kinds::facts(decision, observation, choice);
    };
    let mut reasons = Vec::new();
    let step = decision.next_step;
    if step.is_none() {
        reasons.push(RefusalReason::NoNextStep);
    }
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
    let target = match kinds::chosen_target(repo_facts, choice) {
        Ok(target) => target,
        Err(reason) => {
            reasons.push(reason);
            return Err(Refusal { reasons });
        }
    };
    if target.archived {
        reasons.push(RefusalReason::RepoArchived {
            repo: target.target.repo.clone(),
        });
    }
    // A PR's refs live in the Repo it is in; a same-numbered PR elsewhere is
    // another PR. So its Job targets that Repo, whatever the fork's setting.
    if &target.target.repo != repo {
        reasons.push(RefusalReason::PullRequestNotInTarget {
            repo: repo.clone(),
            number: *number,
            target: target.target.repo.clone(),
        });
    }
    // A head branch in another Repo (or a deleted one) is fetched as the PR
    // ref from the target's remote (#13).
    let base = if pr.head_repo.as_ref() == Some(repo) {
        BaseRef::HeadBranch(pr.head_ref.clone())
    } else {
        BaseRef::PullHead { number: *number }
    };
    match step {
        Some(step) if reasons.is_empty() => Ok(Facts {
            step,
            target: target.target,
            number: Some(*number),
            base,
            base_commit: Some(pr.head_commit.clone()),
            branch: pr.head_ref.clone(),
            fresh: false,
        }),
        _ => Err(Refusal { reasons }),
    }
}

/// What to inspect for launching `decision`'s Work Item, or why den won't even
/// look (no Next Step, archived, …).
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
        worktree_name: names::worktree_name(&facts.branch),
        base_branch: facts
            .base_commit
            .is_none()
            .then(|| facts.base.branch().map(str::to_owned))
            .flatten(),
        branch: facts.branch,
        item: decision.item.id.clone(),
        fresh: facts.fresh,
    })
}

/// Plans the launch, or refuses with every reason found.
pub fn plan(input: &PlanInput<'_>) -> Result<LaunchPlan, Refusal> {
    let facts = facts(input.decision, input.observation, input.choice)?;
    let item = &input.decision.item.id;
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
        .map_err(|why| reasons.extend(why))
        .ok();

    // Starting fresh beside a Live Job (#14) suffixes the worktree name, the
    // local branch and the session, before any other collision is considered.
    let beside_live_job = |name: String, max_chars: usize| match input.fresh {
        Some(n) => names::suffixed_within(&name, n, max_chars),
        None => name,
    };
    // The worktree name: suffixed (again) when other Work Items' Jobs have it.
    let wanted = beside_live_job(
        names::worktree_name(&facts.branch),
        names::MAX_WORKTREE_NAME_CHARS,
    );
    let records = view.jobs.as_ref().map_or(&[][..], Vec::as_slice);
    let named = found.as_ref().and_then(|(clone, _)| {
        let free = names::free_worktree_name(&wanted, &clone.path, item, facts.fresh, records);
        if free.is_none() {
            reasons.push(RefusalReason::NoFreeWorktreeName {
                wanted: wanted.clone(),
            });
        }
        free
    });
    let worktree_name = named
        .as_ref()
        .map_or_else(|| wanted.clone(), |(name, _)| name.clone());
    let branch_name = if facts.fresh {
        worktree_name.clone()
    } else {
        // Beside a Live Job the local branch is new and tracks the same branch
        // on the target's remote.
        beside_live_job(facts.branch.clone(), MAX_FRESH_BRANCH_CHARS)
    };

    let base_commit = found
        .as_ref()
        .and_then(|(clone, remote)| base_commit(&facts, view, clone, remote, &mut reasons));

    let mut branch_start = None;
    let mut writable = Writability::Unknown("not inspected".to_owned());
    match &found {
        Some((clone, remote)) if matches!(clone.source, CloneSource::ToClone { .. }) => {
            // Execution reuses the branch if cloning creates it, detaching
            // only the Main Clone created by this launch before worktree add.
            writable = clone.writable.clone();
            branch_start = Some(new_branch_start(&facts, remote));
        }
        Some((clone, remote)) => {
            let chosen = view.chosen.as_ref().filter(|c| c.path == clone.path);
            match chosen {
                Some(state) if state.worktree_name == worktree_name => {
                    let checked = Checked {
                        facts: &facts,
                        item,
                        view,
                        clone,
                        remote,
                        branch: &branch_name,
                        worktree: worktree_path(&clone.path, &worktree_name),
                        base_commit: base_commit.as_deref(),
                    };
                    writable = check_clone(&checked, state, &mut reasons);
                    branch_start = check_branch(&checked, state, &mut reasons);
                }
                Some(state) => reasons.push(RefusalReason::InspectionFailed {
                    error: format!(
                        "the worktree name {worktree_name} was not inspected ({} was)",
                        state.worktree_name
                    ),
                }),
                None => reasons.push(RefusalReason::InspectionFailed {
                    error: format!("{} was not inspected", clone.path.display()),
                }),
            }
        }
        None => {}
    }

    let (Some((mut clone, remote)), Some(prompts), Some(start), Some(commit), true) = (
        found,
        prompts,
        branch_start,
        base_commit,
        reasons.is_empty(),
    ) else {
        return Err(Refusal { reasons });
    };
    clone.writable = writable;
    let worktree = worktree_path(&clone.path, &worktree_name);
    let decision = input.decision;
    let prompt = prompts.render(
        facts.step,
        &PromptFields {
            repo: facts.target.repo.to_string(),
            number: facts.number,
            url: decision.item.url.clone(),
            title: decision.item.title.clone(),
            branch: branch_name.clone(),
        },
    );
    let session = beside_live_job(
        names::session_name(item, facts.step),
        names::MAX_SESSION_NAME_CHARS,
    );
    let renamed = named
        .filter(|(name, _)| *name != wanted)
        .map(|(_, taken_by)| Renamed { wanted, taken_by });
    let mut plan = LaunchPlan {
        work_item: decision.item.id.clone(),
        title: decision.item.title.clone(),
        url: decision.item.url.clone(),
        next_step: facts.step,
        target: facts.target,
        main_clone: clone,
        remote,
        base: Base {
            reference: facts.base,
            commit,
        },
        branch: JobBranch {
            name: branch_name,
            start,
        },
        worktree,
        renamed,
        session,
        prompt,
        window: view.window.clone(),
        command: String::new(),
    };
    plan.command = command::display(&plan.window, &plan.window_command());
    Ok(plan)
}

/// The commit the Job starts on: the PR head GitHub reported, or where the
/// remote said the base branch is.
fn base_commit(
    facts: &Facts,
    view: &WorkspaceView,
    clone: &MainClone,
    remote: &Remote,
    reasons: &mut Vec<RefusalReason>,
) -> Option<String> {
    if let Some(commit) = &facts.base_commit {
        return Some(commit.clone());
    }
    let from = match &clone.source {
        CloneSource::ToClone { url, .. } => url.clone(),
        _ => remote.name.clone(),
    };
    let Some(branch) = facts.base.branch().map(str::to_owned) else {
        reasons.push(RefusalReason::InspectionFailed {
            error: format!("den has no commit for {}", facts.base.remote_ref()),
        });
        return None;
    };
    match &view.base_tip {
        Some(Ok(Some(commit))) => return Some(commit.clone()),
        Some(Ok(None)) => reasons.push(RefusalReason::BaseBranchGone { from, branch }),
        Some(Err(error)) => reasons.push(RefusalReason::BaseUnreadable {
            from,
            branch,
            error: error.clone(),
        }),
        None => reasons.push(RefusalReason::InspectionFailed {
            error: format!("den didn't ask {from} where {branch} is"),
        }),
    }
    None
}

/// A local branch the Job creates: a fresh `<type>_<slug>` branch tracking
/// nothing, the target's branch tracked under its remote, or a cross-Repo PR
/// head (a PR ref) tracking nothing.
fn new_branch_start(facts: &Facts, remote: &Remote) -> BranchStart {
    match facts.base.branch() {
        _ if facts.fresh => BranchStart::Fresh,
        Some(branch) => BranchStart::New {
            tracking: format!("{}/{branch}", remote.name),
        },
        None => BranchStart::NewUntracked,
    }
}

/// What the existing-clone checks look at.
struct Checked<'a> {
    facts: &'a Facts,
    item: &'a WorkItemId,
    view: &'a WorkspaceView,
    clone: &'a MainClone,
    remote: &'a Remote,
    /// The Job's local branch name (suffixed for a fresh branch).
    branch: &'a str,
    worktree: PathBuf,
    /// `None` when it couldn't be established (already a reason).
    base_commit: Option<&'a str>,
}

/// The Main Clone and the worktree path are usable; returns the clone's writability.
fn check_clone(
    checked: &Checked<'_>,
    state: &ChosenClone,
    reasons: &mut Vec<RefusalReason>,
) -> Writability {
    let clone = checked.clone;
    let writable = match &state.state {
        PathState::Exists { writable } => {
            if *writable == Writability::NotWritable {
                reasons.push(RefusalReason::MainCloneNotWritable {
                    path: clone.path.clone(),
                });
            }
            writable.clone()
        }
        PathState::Missing => {
            reasons.push(RefusalReason::MainCloneMissing {
                path: clone.path.clone(),
            });
            Writability::Unknown("not inspected".to_owned())
        }
        PathState::Unknown(error) => {
            reasons.push(RefusalReason::MainCloneUnknown {
                path: clone.path.clone(),
                error: error.clone(),
            });
            Writability::Unknown("not inspected".to_owned())
        }
    };
    let worktree = &checked.worktree;
    let registered = registered(state, reasons);
    match &state.worktree {
        // Gone from disk but still registered: git would refuse the path.
        PathState::Missing if registered.iter().any(|w| &w.path == worktree) => {
            reasons.push(RefusalReason::WorktreeRegistered {
                path: worktree.clone(),
            });
        }
        PathState::Missing => {}
        PathState::Exists { .. } => reasons.push(RefusalReason::WorktreeExists {
            job: recorded_job(checked.view, checked.item, worktree),
            path: worktree.clone(),
        }),
        PathState::Unknown(error) => reasons.push(RefusalReason::WorktreeUnknown {
            path: worktree.clone(),
            error: error.clone(),
        }),
    }
    writable
}

fn registered<'a>(
    state: &'a ChosenClone,
    reasons: &mut Vec<RefusalReason>,
) -> &'a [RegisteredWorktree] {
    match &state.worktrees {
        Ok(registered) => registered.as_slice(),
        Err(error) => {
            reasons.push(RefusalReason::InspectionFailed {
                error: error.clone(),
            });
            &[]
        }
    }
}

/// How the Job's local branch starts, given what is there locally.
fn check_branch(
    checked: &Checked<'_>,
    state: &ChosenClone,
    reasons: &mut Vec<RefusalReason>,
) -> Option<BranchStart> {
    let branch = checked.branch;
    match &state.local_branch {
        Ok(None) => Some(new_branch_start(checked.facts, checked.remote)),
        Ok(Some(commit)) if Some(commit.as_str()) == checked.base_commit => {
            // Already reported when the list couldn't be read.
            let registered = state.worktrees.as_deref().unwrap_or_default();
            match registered
                .iter()
                .find(|w| w.branch.as_deref() == Some(branch))
            {
                Some(elsewhere) => {
                    reasons.push(RefusalReason::BranchCheckedOut {
                        branch: branch.to_owned(),
                        worktree: elsewhere.path.clone(),
                    });
                    None
                }
                None => Some(BranchStart::Existing),
            }
        }
        Ok(Some(commit)) => {
            if let Some(base) = checked.base_commit {
                reasons.push(RefusalReason::LocalBranchDiffers {
                    branch: branch.to_owned(),
                    commit: commit.clone(),
                    base_name: base_name(checked.facts, checked.remote),
                    base: base.to_owned(),
                });
            }
            None
        }
        Err(error) => {
            reasons.push(RefusalReason::InspectionFailed {
                error: error.clone(),
            });
            None
        }
    }
}

/// The base in words, for refusals: "the PR head", "upstream/main".
fn base_name(facts: &Facts, remote: &Remote) -> String {
    match facts.base.branch() {
        Some(branch) if facts.base_commit.is_none() => format!("{}/{branch}", remote.name),
        _ => "the PR head".to_owned(),
    }
}

/// The Main Clone and its remote for the target (one den will clone, when
/// none qualifies), or why there is none.
fn main_clone(
    facts: &Facts,
    view: &WorkspaceView,
) -> Result<(MainClone, Remote), Vec<RefusalReason>> {
    found_clone(facts, view).or_else(|no_clone| match (&no_clone, &view.new_clone) {
        (
            RefusalReason::NoMainClone {
                search_error: None, ..
            },
            Some(folders),
        ) => {
            // Name the near misses too, so the user sees why none qualified.
            to_clone(&facts.target, folders).map_err(|why| vec![no_clone, why])
        }
        _ => Err(vec![no_clone]),
    })
}

/// The existing Main Clone and its remote for the target, or why there is none.
fn found_clone(facts: &Facts, view: &WorkspaceView) -> Result<(MainClone, Remote), RefusalReason> {
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

/// No Main Clone: the one den will clone, from the target's URL.
fn to_clone(target: &Target, folders: &NewCloneView) -> Result<(MainClone, Remote), RefusalReason> {
    let spot = cloning::choose(folders, &target.repo)?;
    Ok((
        MainClone {
            path: spot.path,
            source: CloneSource::ToClone {
                url: target.url.clone(),
                root: folders.root.clone(),
                taken: spot.taken,
            },
            writable: spot.writable,
        },
        Remote {
            name: cloning::CLONE_REMOTE.to_owned(),
            url: target.url.clone(),
        },
    ))
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
