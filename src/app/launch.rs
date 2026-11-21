//! The launch sheet (#11): `c` opens it on the selected Work Item, den inspects
//! the workspace and plans the launch, the user may edit the prompt, `Enter`
//! launches and `Esc` cancels.
//!
//! Keys on the sheet: `Enter` launch, `Esc` cancel, `e` or `Tab` edit the
//! prompt. While editing, typed keys go into the prompt (`←`/`→`, `Home`/`End`,
//! `Backspace`/`Delete`); `Enter`, `Esc` or `Tab` stop editing. `Ctrl-C` quits
//! from anywhere. `u` (#13), on a Work Item in a fork, switches this Job's
//! target between the fork and its Upstream without changing the fork's own
//! setting (the sidebar's `u`); the sheet looks at the workspace again for the
//! new target. Elsewhere `u` changes nothing and says why.
//!
//! Live Jobs (#14): when the inspection finds a Live Job for the Work Item, the
//! sheet asks "open existing or start fresh?" (`o` / `f`). Open existing plans
//! a window resuming the Job's session (`Enter` opens it); start fresh inspects
//! again for suffixed names and continues as a normal launch. A Job whose
//! worktree state is unknown is explained; only an explicit `f` launches.
//! A Live Job stays reopenable when a fresh launch is refused (for example,
//! the PR has no Next Step while its checks run): the sheet then inspects the
//! Job's own worktree and offers only `o`. `u` works on the question too (a
//! fork's Upstream target refused, a fresh start on the fork wanted): switching
//! the target forgets a fresh start, so the question is asked again for it.
//!
//! Request review (#39): the sheet also lists the reviewers the Job will ask,
//! starting from the Decision's suggested reviewers. `r` edits them (logins
//! and `org/team`s, comma- or space-separated); `Enter`, `Esc` or `Tab` stop
//! editing and rebuild the prompt from its template with the new list. An
//! edited prompt is replaced then, and the sheet says so. `Enter` launches
//! only with a non-empty, well-formed list. The Job is a coordination Job: it
//! runs in the approval-required permission mode, shown on the sheet. A list
//! the user never edited follows the Decision's suggestions when a survey
//! changes them.
//!
//! Linear Issues (#38): `c` on a Start or an Implement Linear Issue opens the
//! same sheet with the Linear writes on it ([`super::linear_writes`]): the
//! claim (Start only: assign to the Linear viewer) and the state move, on by
//! default (`m` turns it off; Implement offers it only when the issue hasn't
//! started). The target Repo is one of the Linear Workflow's Repos for new
//! work: the only one, or none until `t` picks one when there are several.
//! `Enter` re-reads the issue, then writes, then launches; any failure stops
//! there, and nothing is undone.
//!
//! Land (#40): `c` on a Linear Issue whose Next Step is Land opens the Land
//! sheet: the members to land in landing order (Repo, PR, head commit,
//! checks) and the held members as "not landed: held". The Job is a
//! coordination Job on the first member's head.
//!
//! Every ready sheet lists the slash commands its prompt invokes, as den
//! found them in the user's and the target Repo's Claude command and skill
//! directories, or "not found" with where den looked (#41). The inspection
//! reads the directories once; an edited prompt is checked against the same
//! read.
//!
//! `Enter` plans again from the current Decision. When a survey that landed
//! while the sheet was open changed what decides the Job (its Next Step, and
//! so its permission mode and prompt template; its reviewers; a Landing
//! Group's members, heads or held members; den's prompt), nothing is
//! launched: the sheet shows the new plan, says what changed, and asks again
//! (spec 0001, "Re-verification"). Editing the reviewers checks
//! the shown plan the same way before it adopts the new list. The prompt and
//! reviewer checks come after, so they always speak about the plan shown.

use super::{
    App, Effect, Key, LogLevel, OverlayState,
    editor::{Edit, PromptEditor},
    event::LaunchId,
    linear_writes::{self, Next, Shown, Watch, Writes},
};
use crate::{
    decide::Decision,
    domain::{LinearWorkflowPolicy, NextStep, Observation, RepoId, WorkItemId, facts::Reviewer},
    handoff::{
        self, InspectRequest, JobCheck, LaunchOutcome, LaunchPlan, LaunchReport, LaunchSettings,
        PlanInput, ProjectSettings, Refusal, RefusalReason, ReopenPlan, TargetChoice,
        WorkspaceView, WorktreeState, check_prompt, live, names, reviewers, short_commit, trust,
        workflows::WorkflowDirs,
    },
    jobs::JobRecord,
    ports::{LinearIssueRead, LinearIssueRequest, LinearWriteKind, LinearWriteOutcome},
};

/// One open launch sheet.
#[derive(Debug, Clone)]
pub(super) struct Sheet {
    pub id: LaunchId,
    pub item: WorkItemId,
    /// What the sheet calls the Work Item: `owner/name#12`, or a Linear
    /// Issue's identifier.
    pub reference: String,
    pub title: String,
    /// The Next Step when the sheet opened; once there is a plan, the sheet
    /// shows the plan's instead.
    pub next_step: Option<NextStep>,
    pub state: SheetState,
    pub prompt: PromptEditor,
    pub editing: bool,
    /// A one-off message, e.g. why `Enter` did nothing.
    pub notice: Option<String>,
    /// The PR head commit the user accepted after it moved.
    pub accepted_head: Option<String>,
    /// This Job's target: the Work Item's Repo or, for a fork, its Upstream.
    pub choice: TargetChoice,
    /// The target the sheet opened on: the fork's own setting for an Issue or
    /// a Run, always the Work Item's own Repo for a PR or a Branch
    /// ([`TargetChoice::default_for`]). `choice` differs from it only after
    /// `u` on the sheet (a one-Job override).
    pub fork_choice: TargetChoice,
    /// The Work Item's Repo, when it is a fork (so `u` means something), and
    /// its Upstream if den can see it.
    pub fork: Option<ForkTargets>,
    /// The prompt the user edited before switching the target with `u`; kept
    /// instead of the new target's default prompt.
    pub edited_prompt: Option<String>,
    /// The current target's default prompt, as the last inspection planned it.
    /// A launch attempt puts the edited prompt into the plan, so the plan's
    /// own prompt can't tell an edit from the default.
    pub default_prompt: Option<String>,
    /// Start fresh (#14): the suffix for the new worktree, branch and session,
    /// and the Job it starts beside.
    pub fresh: Option<(u32, Box<JobCheck>)>,
    /// Why a fresh launch can't happen, when the sheet was opened only to
    /// reopen the Work Item's Live Job (#14). `f` is not offered then.
    pub launch_refused: Option<Refusal>,
    /// Request review (#39): the reviewers the Job will ask, editable with `r`.
    pub reviewers: Option<ReviewersField>,
    /// A Linear Issue's Start or Implement (#38): its Linear writes and the
    /// Repos offered for new work.
    pub linear: Option<LinearLaunch>,
    /// The Claude directories the last inspection read (#41): the sheet
    /// checks whatever prompt it shows, edited or not, against them.
    pub workflows: Option<WorkflowDirs>,
}

/// A Linear Issue's Start or Implement sheet (#38).
#[derive(Debug, Clone)]
pub(super) struct LinearLaunch {
    pub writes: Writes,
    /// What the survey last said of the issue, as the sheet synced it: a
    /// survey that changes it while the sheet is open makes `Enter` ask again.
    observed: Shown,
    /// The Linear Workflow's Repos for new work.
    pub repos: Vec<RepoId>,
    /// The chosen one, by index; `None` until `t` picks one of several.
    pub repo: Option<usize>,
    /// Linear's suggested branch name, as the issue carries it.
    pub suggested_branch: Option<String>,
    /// The Linear Workflow's in-progress state, if it names one.
    in_progress_state: Option<String>,
}

impl LinearLaunch {
    pub fn repo(&self) -> Option<&RepoId> {
        self.repos.get(self.repo?)
    }

    /// The claim and the move follow the Next Step: Start claims; Start
    /// moves, and Implement moves only an issue that hasn't started.
    fn follow(&mut self, step: Option<NextStep>) {
        let start = step == Some(NextStep::Start);
        self.writes.claim = start;
        let offered = start || linear_writes::not_started(&self.writes.shown.state);
        self.writes.move_to = self.in_progress_state.clone().filter(|_| offered);
    }
}

/// A Request review sheet's reviewers (#39).
#[derive(Debug, Clone)]
pub(super) struct ReviewersField {
    /// Logins and `org/team`s, as the user edits them.
    pub editor: PromptEditor,
    pub editing: bool,
    /// The Decision's suggested reviewers, as the sheet last planned with.
    pub suggested: Vec<Reviewer>,
    /// What den did not read to filter the suggestions (#47's
    /// [`Decision::suggested_reviewers_unread`]), as the sheet last planned
    /// with: they may name someone already asked or approved.
    pub suggested_unread: Option<String>,
}

impl ReviewersField {
    fn new(decision: &Decision) -> Self {
        let suggested = decision.suggested_reviewers.clone().unwrap_or_default();
        Self {
            editor: PromptEditor::new(&reviewers::spell_list(&suggested)),
            editing: false,
            suggested,
            suggested_unread: decision.suggested_reviewers_unread.clone(),
        }
    }

    /// The list as typed, parsed.
    fn chosen(&self) -> Result<Vec<Reviewer>, reviewers::ReviewersProblem> {
        reviewers::parse_list(&self.editor.text())
    }

    /// The list still reads as the suggestions: the user never made it theirs.
    fn follows_suggestions(&self) -> bool {
        self.editor.text() == reviewers::spell_list(&self.suggested)
    }
}

/// The two targets a Work Item in a fork can have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ForkTargets {
    pub fork: RepoId,
    pub upstream: Option<RepoId>,
    /// The fork's own setting (sidebar `u`) targets its Upstream.
    pub set_to_upstream: bool,
}

#[derive(Debug, Clone)]
pub(super) enum SheetState {
    /// A Linear Issue's sheet waits for `t` to pick one of several Repos (#38).
    ChooseRepo,
    Inspecting,
    Ready {
        plan: Box<LaunchPlan>,
        view: Box<WorkspaceView>,
        /// The commit the head moved from, when it moved since the sheet opened.
        moved_from: Option<String>,
    },
    Refused(Refusal),
    /// Confirmed: den is re-reading and writing to Linear before it
    /// launches (#38).
    Writing {
        plan: Box<LaunchPlan>,
        view: Box<WorkspaceView>,
    },
    /// The Linear writes stopped (a failure, an unknown outcome): nothing was
    /// launched and nothing more will be done (#38).
    Stopped {
        plan: Box<LaunchPlan>,
    },
    Launching {
        plan: Box<LaunchPlan>,
        view: Box<WorkspaceView>,
    },
    Finished {
        plan: Box<LaunchPlan>,
        report: Box<LaunchReport>,
    },
    /// The Work Item has a Live Job: open existing (`o`) or start fresh (`f`)?
    LiveJob {
        check: Box<JobCheck>,
        view: Box<WorkspaceView>,
    },
    /// The Work Item's Job has a worktree den can't tell about: explained,
    /// never treated as gone; only `f` starts fresh.
    JobUnknown {
        check: Box<JobCheck>,
        view: Box<WorkspaceView>,
    },
    /// Open existing is planned: `Enter` opens the window.
    ReopenReady {
        plan: Box<ReopenPlan>,
    },
    Reopening {
        plan: Box<ReopenPlan>,
    },
    Reopened {
        plan: Box<ReopenPlan>,
        report: Box<LaunchReport>,
    },
}

impl App {
    /// `c`: opens the launch sheet on the selected Work Item.
    pub(super) fn open_launch_sheet(&mut self) -> Vec<Effect> {
        let Some(decision) = self.selected_index().and_then(|i| self.decisions.get(i)) else {
            return Vec::new();
        };
        // Mark for release is a Linear write den makes itself, never a Job.
        if decision.next_step == Some(NextStep::MarkForRelease) {
            let decision = decision.clone();
            return self.open_release_sheet(&decision);
        }
        let id = LaunchId(self.next_launch);
        self.next_launch += 1;
        let linear = self.linear_launch(decision);
        let repo = match &linear {
            Some(linear) => linear.repo().cloned(),
            None => decision.item.id.repo().cloned(),
        };
        let fork = repo.and_then(|repo| self.fork_targets(&repo));
        // A PR's or Branch's Job starts on its own Repo whatever the fork's
        // setting; Issues, Runs and Linear Issues follow it.
        let fork_choice = TargetChoice::default_for(&decision.item.id, fork_setting(fork.as_ref()));
        let mut sheet = Sheet {
            id,
            item: decision.item.id.clone(),
            reference: match &decision.item.id {
                WorkItemId::LinearIssue { .. } => decision.item.reference(),
                other => other.to_string(),
            },
            title: decision.item.title.clone(),
            next_step: decision.next_step,
            state: SheetState::Inspecting,
            prompt: PromptEditor::default(),
            editing: false,
            notice: None,
            accepted_head: None,
            choice: fork_choice,
            fork_choice,
            fork,
            edited_prompt: None,
            default_prompt: None,
            fresh: None,
            launch_refused: None,
            reviewers: (decision.next_step == Some(NextStep::RequestReview))
                .then(|| ReviewersField::new(decision)),
            linear,
            workflows: None,
        };
        let effects = self.inspect_for(&mut sheet, "opened");
        self.overlay = Some(OverlayState::Launch(Box::new(sheet)));
        effects
    }

    /// A Work Item's Repo's two targets, when it is a fork.
    fn fork_targets(&self, repo: &RepoId) -> Option<ForkTargets> {
        let repo = self.shown.as_ref()?.observation.repo(repo)?;
        repo.is_fork.then(|| ForkTargets {
            fork: repo.id.clone(),
            upstream: repo.upstream().map(|u| u.id.clone()),
            set_to_upstream: self.upstreams.contains(&repo.id),
        })
    }

    /// The Linear part of a Start or Implement sheet on a Linear Issue (#38);
    /// `None` for any other Work Item or Next Step.
    fn linear_launch(&self, decision: &Decision) -> Option<LinearLaunch> {
        if !matches!(
            decision.next_step,
            Some(NextStep::Start | NextStep::Implement)
        ) {
            return None;
        }
        let (writes, workflow) = self.linear_writes_for(decision, Watch::AssigneeAndState)?;
        let issue = self.linear_issue(&decision.item.id)?;
        let repos = workflow.new_work_repos.clone();
        let mut linear = LinearLaunch {
            observed: writes.shown.clone(),
            writes,
            repo: (repos.len() == 1).then_some(0),
            repos,
            suggested_branch: issue.branch_name.clone(),
            in_progress_state: workflow.in_progress_state.clone(),
        };
        linear.follow(decision.next_step);
        Some(linear)
    }

    /// The Linear Issue `item` is, as den last observed it.
    pub(super) fn linear_issue(
        &self,
        item: &WorkItemId,
    ) -> Option<&crate::domain::facts::LinearIssue> {
        self.shown
            .as_ref()?
            .observation
            .linear
            .issues
            .iter()
            .find(|i| i.work_item_id().same(item))
    }

    /// The writes a sheet on `decision`'s Linear Issue can make, as the
    /// survey showed the issue (nothing chosen yet), and its Linear Workflow.
    pub(super) fn linear_writes_for(
        &self,
        decision: &Decision,
        watch: Watch,
    ) -> Option<(Writes, crate::config::LinearWorkflow)> {
        let issue = self.linear_issue(&decision.item.id)?;
        let workflow = self.config.as_ref().ok()?.linear_workflow(&issue.owner)?;
        let viewer = self.shown.as_ref().and_then(|shown| {
            shown
                .observation
                .linear
                .viewers
                .iter()
                .find(|v| {
                    v.owner.eq_ignore_ascii_case(&issue.owner) && v.workspace == issue.id.workspace
                })
                .cloned()
        });
        let request = LinearIssueRequest {
            workflow: workflow.clone(),
            issue: issue.id.clone(),
            identifier: issue.identifier.clone(),
        };
        let shown = Shown {
            assignee: issue.assignee.clone(),
            state: issue.state.clone(),
        };
        Some((Writes::new(request, viewer, shown, watch), workflow.clone()))
    }

    /// Asks to inspect the workspace for the sheet's Work Item and target, or
    /// refuses straight away when den won't even look. A refused launch still
    /// inspects the Work Item's Live Job when den last saw one (#14), so it
    /// stays reopenable.
    fn inspect_for(&self, sheet: &mut Sheet, what: &str) -> Vec<Effect> {
        let gone = |sheet: &mut Sheet| {
            let refusal = Refusal::one(handoff::RefusalReason::WorkItemGone);
            let log = refused_log(sheet.id, &sheet.item, &refusal);
            sheet.state = SheetState::Refused(refusal);
            vec![log]
        };
        let Some(observation) = self.shown.as_ref().map(|s| &s.observation) else {
            return gone(sheet);
        };
        let Some(decision) = self.decision_of(&sheet.item) else {
            return gone(sheet);
        };
        let settings = self.launch_settings();
        sheet.launch_refused = None;
        let linear_repo = sheet.linear.as_ref().and_then(LinearLaunch::repo);
        let request = match &sheet.linear {
            Some(linear) if linear.repos.is_empty() => {
                Err(Refusal::one(RefusalReason::NoNewWorkRepos {
                    owner: sheet.item.owner().to_owned(),
                }))
            }
            Some(linear) if linear.repo.is_none() => {
                sheet.state = SheetState::ChooseRepo;
                return Vec::new();
            }
            _ => handoff::inspect_request(
                decision,
                observation,
                sheet.choice,
                &settings,
                linear_repo,
                self.policies.for_owner(sheet.item.owner()),
            ),
        };
        let request = match request {
            Ok(request) => Ok((
                format!(
                    "launch sheet {} {what} for {}, targeting {}",
                    sheet.id.0, sheet.item, request.target
                ),
                request,
            )),
            // A fresh launch is refused, but a Live Job stays reopenable: the
            // sheet inspects the Job's own worktree and offers only `o`.
            Err(refusal) => match live_job_request(&self.job_checks, &sheet.item, observation) {
                Some(request) => {
                    let log = format!(
                        "launch sheet {} {what} for {} to reopen its Live Job; a fresh launch is refused: {refusal}",
                        sheet.id.0, sheet.item
                    );
                    sheet.launch_refused = Some(refusal);
                    Ok((log, request))
                }
                None => Err(refusal),
            },
        };
        match request {
            Ok((log, request)) => {
                sheet.state = SheetState::Inspecting;
                vec![
                    Effect::log(LogLevel::Info, log),
                    Effect::InspectLaunch {
                        id: sheet.id,
                        request: Box::new(request),
                    },
                ]
            }
            Err(refusal) => {
                let log = refused_log(sheet.id, &sheet.item, &refusal);
                sheet.state = SheetState::Refused(refusal);
                vec![log]
            }
        }
    }

    /// `u` on the sheet: this Job's target switches between the fork and its
    /// Upstream; the fork's own setting stays as it is.
    fn switch_target(&mut self, sheet: &mut Sheet) -> Vec<Effect> {
        let Some(fork) = &sheet.fork else {
            let repo = match &sheet.linear {
                Some(linear) => linear.repo().cloned(),
                None => sheet.item.repo().cloned(),
            };
            sheet.notice = Some(match repo {
                Some(repo) => {
                    format!(
                        "u does nothing here: {repo} is not a fork, so it has no Upstream to target"
                    )
                }
                None => {
                    "u does nothing here: no target Repo is chosen yet (t chooses one)".to_owned()
                }
            });
            return Vec::new();
        };
        if sheet.choice == TargetChoice::OwnRepo && fork.upstream.is_none() {
            sheet.notice = Some(format!(
                "u does nothing here: den can't see {}'s Upstream",
                fork.fork
            ));
            return Vec::new();
        }
        if let SheetState::Ready { plan, .. } = &sheet.state {
            // The prompt as it stands now wins: an edit undone back to the
            // default clears the saved one, so it can't come back later.
            let text = sheet.prompt.text();
            let default = sheet.default_prompt.as_deref().unwrap_or(&plan.prompt);
            sheet.edited_prompt = (text != default).then_some(text);
        }
        sheet.choice = sheet.choice.flipped();
        sheet.accepted_head = None;
        // A fresh start was picked for the old target's names: ask again.
        sheet.fresh = None;
        // A new id: an inspection still running for the old target is ignored.
        sheet.id = LaunchId(self.next_launch);
        self.next_launch += 1;
        self.inspect_for(sheet, "switched target")
    }

    /// Keys while the launch sheet is open; it claims all of them.
    pub(super) fn launch_key(&mut self, mut sheet: Box<Sheet>, key: Key) -> Vec<Effect> {
        if key == Key::Interrupt {
            return vec![Effect::Quit];
        }
        sheet.notice = None;
        if let Some(field) = sheet.reviewers.as_mut().filter(|f| f.editing) {
            if field.editor.key(key) == Edit::NotAnEdit
                && matches!(key, Key::Enter | Key::Esc | Key::Tab)
            {
                field.editing = false;
                let effects = self.reviewers_edited(&mut sheet);
                self.overlay = Some(OverlayState::Launch(sheet));
                return effects;
            }
            self.overlay = Some(OverlayState::Launch(sheet));
            return Vec::new();
        }
        if sheet.editing {
            if sheet.prompt.key(key) == Edit::NotAnEdit
                && matches!(key, Key::Enter | Key::Esc | Key::Tab)
            {
                sheet.editing = false;
            }
            self.overlay = Some(OverlayState::Launch(sheet));
            return Vec::new();
        }
        let mut effects = Vec::new();
        match (key, &sheet.state) {
            (Key::Esc, SheetState::Launching { .. } | SheetState::Reopening { .. }) => {
                sheet.notice = Some("den is launching; wait for it to finish".to_owned());
            }
            (Key::Esc | Key::Enter, SheetState::Writing { .. }) => {
                sheet.notice = Some("den is writing to Linear; wait for it to finish".to_owned());
            }
            (Key::Enter, SheetState::ChooseRepo) => {
                sheet.notice = Some(RefusalReason::NewWorkRepoNotChosen.to_string());
            }
            (
                Key::Char('t'),
                SheetState::ChooseRepo
                | SheetState::Ready { .. }
                | SheetState::Refused(_)
                | SheetState::Inspecting,
            ) => effects = self.next_repo(&mut sheet),
            (Key::Char('m'), SheetState::Ready { .. }) => toggle_move(&mut sheet),
            (Key::Esc, _)
            | (
                Key::Enter,
                SheetState::Refused(_)
                | SheetState::Finished { .. }
                | SheetState::Reopened { .. }
                | SheetState::Stopped { .. },
            ) => {
                // Closed: nothing was or will be done by this sheet any more.
                return Vec::new();
            }
            (Key::Char('e') | Key::Tab, SheetState::Ready { .. }) => sheet.editing = true,
            (Key::Char('r'), SheetState::Ready { .. }) => match sheet.reviewers.as_mut() {
                Some(field) => field.editing = true,
                None => {
                    sheet.notice = Some(
                        "r does nothing here: only a Request review asks reviewers".to_owned(),
                    );
                }
            },
            (
                Key::Char('u'),
                SheetState::Ready { .. }
                | SheetState::Refused(_)
                | SheetState::Inspecting
                | SheetState::LiveJob { .. }
                | SheetState::JobUnknown { .. },
            ) => effects = self.switch_target(&mut sheet),
            (Key::Enter, SheetState::Ready { .. }) => effects = self.launch(&mut sheet),
            (Key::Char('o'), SheetState::LiveJob { check, view }) => {
                let (check, view) = (check.clone(), view.clone());
                let settings = self.reopen_settings(&check.job);
                let policy = self.policies.for_owner(check.job.owner());
                effects = open_existing(
                    &mut sheet,
                    &check,
                    &view,
                    policy,
                    settings,
                    &self.removed_env,
                );
            }
            (
                Key::Char('f'),
                SheetState::LiveJob { check, view } | SheetState::JobUnknown { check, view },
            ) => match &sheet.launch_refused {
                // Opened only to reopen: say why, and keep the question open.
                Some(refusal) => sheet.notice = Some(format!("can't start fresh: {refusal}")),
                None => {
                    let (check, view) = (check.clone(), view.clone());
                    effects = self.start_fresh(&mut sheet, check, &view);
                }
            },
            (Key::Enter, SheetState::ReopenReady { plan })
                if plan.project_settings != self.reopen_settings(&plan.job) =>
            {
                let mut plan = plan.clone();
                plan.project_settings = self.reopen_settings(&plan.job);
                plan.command = handoff::command::display(&plan.window, &plan.window_command());
                sheet.state = SheetState::ReopenReady { plan };
                sheet.notice = Some(
                    "Not reopened: project settings changed; check the command and press Enter again"
                        .to_owned(),
                );
            }
            (Key::Enter, SheetState::ReopenReady { plan }) => {
                let plan = plan.clone();
                effects = vec![
                    Effect::log(
                        LogLevel::Info,
                        format!(
                            "launch sheet {} reopening {} in {} (session {})",
                            sheet.id.0,
                            sheet.item,
                            plan.job.worktree.display(),
                            plan.job.session
                        ),
                    ),
                    Effect::Reopen {
                        id: sheet.id,
                        plan: plan.clone(),
                    },
                ];
                sheet.state = SheetState::Reopening { plan };
            }
            _ => {}
        }
        self.overlay = Some(OverlayState::Launch(sheet));
        effects
    }

    /// `Enter` on a ready sheet: plans again against what den knows now (a
    /// refresh may have archived the Repo), checks the prompt and the
    /// reviewers, and launches. When a survey that landed while the sheet was
    /// open changed what decides the Job (its Next Step, and so its permission
    /// mode and prompt template; its reviewers; its default prompt), den
    /// launches nothing: the sheet shows the new plan and `Enter` asks again.
    /// That comes before the prompt and reviewer checks, so a problem is
    /// always explained on the plan it is about.
    fn launch(&self, sheet: &mut Sheet) -> Vec<Effect> {
        let SheetState::Ready {
            plan: shown,
            view,
            moved_from,
        } = &sheet.state
        else {
            return Vec::new();
        };
        let (shown, view, moved_from) = (shown.clone(), view.clone(), moved_from.clone());
        // A different first Land member needs its own clone inspection before planning.
        if shown.landing.is_some()
            && let (Some(decision), Some(observed)) =
                (self.decision_of(&sheet.item), self.shown.as_ref())
        {
            let settings = self.launch_settings();
            let request = match handoff::inspect_request(
                decision,
                &observed.observation,
                sheet.choice,
                &settings,
                sheet.linear.as_ref().and_then(LinearLaunch::repo),
                self.policies.for_owner(sheet.item.owner()),
            ) {
                Ok(request) => request,
                Err(refusal) => return refuse(sheet, refusal),
            };
            if !request.target.same(&shown.target.repo) {
                sheet.notice = Some(format!(
                    "Not launched: the Landing Group target changed from {} to {}. The prompt is rebuilt; check the updated sheet, then press Enter to launch",
                    shown.target.repo, request.target
                ));
                sheet.accepted_head = None;
                sheet.edited_prompt = None;
                sheet.fresh = None;
                return self.inspect_for(sheet, "Landing Group target changed");
            }
        }
        sync_reviewers(sheet, self.decision_of(&sheet.item));
        let planned = planned_reviewers(sheet, Some(&shown));
        // A Linear Issue's claim and move follow the survey too (#38).
        let linear_change = self.sync_linear(sheet);
        let plan = match self.replan(sheet, &view, planned.as_deref()) {
            Ok(plan) => plan,
            Err(refusal) => return refuse(sheet, refusal),
        };
        if let Some(change) =
            what_changed(&shown, sheet.default_prompt.as_deref(), &plan).or(linear_change)
        {
            return ask_again(sheet, plan, view, moved_from, &change);
        }
        let text = sheet.prompt.text();
        if let Err(problem) = check_prompt(&text) {
            sheet.notice = Some(format!("{problem}; edit it with e"));
            return Vec::new();
        }
        if let Some(Err(problem)) = sheet.reviewers.as_ref().map(ReviewersField::chosen) {
            sheet.notice = Some(format!("{problem}; edit the reviewers with r"));
            return Vec::new();
        }
        let plan = match plan.with_prompt(&text) {
            Ok(plan) => plan,
            Err(problem) => {
                sheet.notice = Some(format!("{problem}; edit it with e"));
                return Vec::new();
            }
        };
        // Linear writes first (#38): re-read, assign, move; the launch
        // follows only once every write is done.
        if let Some(linear) = sheet.linear.as_mut()
            && let Next::Wait(effects) = linear.writes.begin(sheet.id)
        {
            sheet.state = SheetState::Writing {
                plan: Box::new(plan),
                view,
            };
            return effects;
        }
        let log = Effect::log(
            LogLevel::Info,
            format!(
                "launch {} confirmed for {}: {} from {} at {} into {}",
                sheet.id.0,
                plan.work_item,
                plan.target.repo,
                plan.remote.name,
                short_commit(&plan.base.commit),
                plan.worktree.display()
            ),
        );
        let plan = Box::new(plan);
        sheet.state = SheetState::Launching {
            plan: plan.clone(),
            view,
        };
        vec![log, Effect::Launch { id: sheet.id, plan }]
    }

    /// `f` on a Live (or unknown) Job: picks a suffix no Job or worktree uses,
    /// then inspects again for the suffixed worktree and branch. The old
    /// worktree is left as it is.
    fn start_fresh(
        &self,
        sheet: &mut Sheet,
        beside: Box<JobCheck>,
        view: &WorkspaceView,
    ) -> Vec<Effect> {
        let gone = || Refusal::one(RefusalReason::WorkItemGone);
        let request = self
            .decision_of(&sheet.item)
            .ok_or_else(gone)
            .and_then(|decision| {
                let observation = self
                    .shown
                    .as_ref()
                    .map(|s| &s.observation)
                    .ok_or_else(gone)?;
                let settings = self.launch_settings();
                handoff::inspect_request(
                    decision,
                    observation,
                    sheet.choice,
                    &settings,
                    sheet.linear.as_ref().and_then(LinearLaunch::repo),
                    self.policies.for_owner(sheet.item.owner()),
                )
            })
            .and_then(|request| {
                live::fresh_suffix(view, &request.worktree_name, &request.branch)
                    .map(|n| (n, request.fresh(n)))
                    .ok_or_else(|| {
                        Refusal::one(RefusalReason::NoFreshName {
                            last_tried: names::MAX_NAME_SUFFIX,
                        })
                    })
            });
        match request {
            Ok((n, request)) => {
                let log = Effect::log(
                    LogLevel::Info,
                    format!(
                        "launch sheet {} starting fresh for {} as {} beside {}",
                        sheet.id.0,
                        sheet.item,
                        request.worktree_name,
                        beside.job.worktree.display()
                    ),
                );
                sheet.fresh = Some((n, beside));
                sheet.state = SheetState::Inspecting;
                vec![
                    log,
                    Effect::InspectLaunch {
                        id: sheet.id,
                        request: Box::new(request),
                    },
                ]
            }
            Err(refusal) => {
                let log = refused_log(sheet.id, &sheet.item, &refusal);
                sheet.state = SheetState::Refused(refusal);
                vec![log]
            }
        }
    }

    /// The reviewers were edited (`r`, then `Enter`/`Esc`/`Tab`): a list that
    /// parses rebuilds the prompt from its template; one that doesn't is
    /// explained and changes nothing else. Before adopting the new list, den
    /// checks the plan the sheet shows as `Enter` does: when a survey changed
    /// what decides the Job, the sheet shows the new plan and says what
    /// changed, never swaps it in silently.
    fn reviewers_edited(&self, sheet: &mut Sheet) -> Vec<Effect> {
        let Some(field) = &sheet.reviewers else {
            return Vec::new();
        };
        let chosen = match field.chosen() {
            Ok(chosen) => chosen,
            Err(problem) => {
                sheet.notice = Some(format!("{problem}; edit the reviewers with r"));
                return Vec::new();
            }
        };
        let SheetState::Ready {
            plan: shown,
            view,
            moved_from,
        } = &sheet.state
        else {
            return Vec::new();
        };
        if shown
            .review_request
            .as_ref()
            .is_some_and(|r| r.reviewers == chosen)
        {
            return Vec::new();
        }
        let (shown, view, moved_from) = (shown.clone(), view.clone(), moved_from.clone());
        sync_reviewers(sheet, self.decision_of(&sheet.item));
        // The plan the sheet shows, as den would make it now: with the
        // reviewers it shows, so only what a survey changed differs.
        let as_shown = shown.review_request.as_ref().map(|r| r.reviewers.clone());
        let change = match self.replan(sheet, &view, as_shown.as_deref()) {
            Ok(now) => what_changed(&shown, sheet.default_prompt.as_deref(), &now),
            Err(refusal) => return refuse(sheet, refusal),
        };
        let planned = planned_reviewers(sheet, Some(&shown));
        let plan = match self.replan(sheet, &view, planned.as_deref()) {
            Ok(plan) => plan,
            Err(refusal) => return refuse(sheet, refusal),
        };
        if let Some(change) = change {
            return ask_again(sheet, plan, view, moved_from, &change);
        }
        let default = sheet.default_prompt.as_deref().unwrap_or(&shown.prompt);
        if sheet.prompt.text() != default {
            sheet.notice = Some(
                "The reviewers changed, so den rebuilt the prompt from its template; your edits to the prompt were replaced".to_owned(),
            );
        }
        sheet.edited_prompt = None;
        show_plan(sheet, plan, view, moved_from);
        Vec::new()
    }

    /// `t` on a Linear Issue's sheet: the next of the Linear Workflow's Repos
    /// for new work becomes the target; the sheet looks at the workspace
    /// again for it.
    fn next_repo(&mut self, sheet: &mut Sheet) -> Vec<Effect> {
        let Some(linear) = sheet.linear.as_mut() else {
            sheet.notice = Some(
                "t does nothing here: only a Linear Issue's sheet chooses its Repo".to_owned(),
            );
            return Vec::new();
        };
        if linear.repos.len() < 2 && linear.repo.is_some() {
            sheet.notice = Some(
                "t does nothing here: the Linear Workflow offers only this Repo for new work"
                    .to_owned(),
            );
            return Vec::new();
        }
        if linear.repos.is_empty() {
            return Vec::new();
        }
        linear.repo = Some(linear.repo.map_or(0, |i| (i + 1) % linear.repos.len()));
        let repo = linear.repo().cloned();
        if let SheetState::Ready { plan, .. } = &sheet.state {
            // The prompt as it stands now wins, as for `u`.
            let text = sheet.prompt.text();
            let default = sheet.default_prompt.as_deref().unwrap_or(&plan.prompt);
            sheet.edited_prompt = (text != default).then_some(text);
        }
        sheet.fork = repo.and_then(|repo| self.fork_targets(&repo));
        sheet.fork_choice =
            TargetChoice::default_for(&sheet.item, fork_setting(sheet.fork.as_ref()));
        sheet.choice = sheet.fork_choice;
        sheet.accepted_head = None;
        sheet.fresh = None;
        // A new id: an inspection still running for the old Repo is ignored.
        sheet.id = LaunchId(self.next_launch);
        self.next_launch += 1;
        self.inspect_for(sheet, "chose a Repo")
    }

    /// Brings a Linear Issue sheet in step with the current survey: the claim
    /// and the move follow the Next Step, and an issue the survey now shows
    /// with another assignee or state is shown so, and `Enter` asks again.
    fn sync_linear(&self, sheet: &mut Sheet) -> Option<String> {
        let step = self.decision_of(&sheet.item).and_then(|d| d.next_step);
        let issue = self.linear_issue(&sheet.item).cloned();
        let linear = sheet.linear.as_mut()?;
        if linear.writes.finished {
            return None;
        }
        let mut change = None;
        if let Some(issue) = issue {
            let now = Shown {
                assignee: issue.assignee,
                state: issue.state,
            };
            if now != linear.observed {
                change = Some(format!(
                    "the survey now shows {} {}",
                    linear.writes.request.identifier,
                    linear_writes::describe(now.assignee.as_ref(), &now.state)
                ));
                linear.writes.shown = now.clone();
                linear.observed = now;
            }
        }
        let (claim, move_to) = (linear.writes.claim, linear.writes.move_to.clone());
        linear.follow(step);
        if change.is_none()
            && (claim, move_to) != (linear.writes.claim, linear.writes.move_to.clone())
        {
            change = Some("the Linear writes changed with the Next Step".to_owned());
        }
        change
    }

    /// The Linear Issue was read again for sheet `id` (#38).
    pub(super) fn linear_issue_read(&mut self, id: LaunchId, read: LinearIssueRead) -> Vec<Effect> {
        self.linear_step(id, "re-read", |writes| writes.reread(id, read))
    }

    /// A Linear write for sheet `id` ended (#38).
    pub(super) fn linear_write_finished(
        &mut self,
        id: LaunchId,
        write: LinearWriteKind,
        outcome: LinearWriteOutcome,
    ) -> Vec<Effect> {
        self.linear_step(id, "Linear write", |writes| {
            writes.written(id, write, outcome)
        })
    }

    /// A Linear re-read or write for sheet `id` couldn't be queued.
    pub(super) fn linear_not_queued(
        &mut self,
        id: LaunchId,
        effect: &Effect,
        reason: &str,
    ) -> Vec<Effect> {
        self.linear_step(id, "Linear request", |writes| {
            writes.not_queued(id, effect, reason)
        })
    }

    /// Advances the Linear writes of sheet `id`, a Start or Implement sheet
    /// or the Mark for release sheet, by `step`.
    fn linear_step(
        &mut self,
        id: LaunchId,
        what: &str,
        step: impl FnOnce(&mut Writes) -> Next,
    ) -> Vec<Effect> {
        if self.release_sheet_is(id) {
            return self.release_step(id, step);
        }
        let Some(mut sheet) = self.current_sheet(id) else {
            return vec![stale(id, what)];
        };
        let (SheetState::Writing { plan, view }, Some(linear)) =
            (sheet.state.clone(), sheet.linear.as_mut())
        else {
            self.overlay = Some(OverlayState::Launch(sheet));
            return vec![stale(id, what)];
        };
        let effects = match step(&mut linear.writes) {
            Next::Wait(effects) => effects,
            Next::Done => {
                let log = Effect::log(
                    LogLevel::Info,
                    format!(
                        "launch {} Linear writes done for {}; launching into {}",
                        id.0,
                        sheet.item,
                        plan.worktree.display()
                    ),
                );
                sheet.state = SheetState::Launching {
                    plan: plan.clone(),
                    view,
                };
                vec![log, Effect::Launch { id, plan }]
            }
            Next::AskAgain(change) => {
                sheet.notice = Some(format!(
                    "Nothing written or launched: {change} since the sheet showed it. Check the sheet, then press Enter to write and launch"
                ));
                sheet.state = SheetState::Ready {
                    plan,
                    view,
                    moved_from: None,
                };
                vec![Effect::log(
                    LogLevel::Info,
                    format!(
                        "launch sheet {} for {}: {change}; asking again",
                        id.0, sheet.item
                    ),
                )]
            }
            Next::Stopped => {
                sheet.state = SheetState::Stopped { plan };
                vec![Effect::log(
                    LogLevel::Warn,
                    format!(
                        "launch sheet {} for {}: the Linear writes stopped; nothing launched",
                        id.0, sheet.item
                    ),
                )]
            }
        };
        self.overlay = Some(OverlayState::Launch(sheet));
        effects
    }

    /// The Linear Workflow of `item`'s Owner, if it has one.
    fn linear_workflow_of(&self, item: &WorkItemId) -> Option<&crate::config::LinearWorkflow> {
        let repo = item.repo()?;
        self.config.as_ref().ok()?.linear_workflow(&repo.owner)
    }

    /// Whether reopening `job` loads the worktree's project settings, decided
    /// again from the survey shown now ([`trust::for_reopen`]).
    fn reopen_settings(&self, job: &JobRecord) -> ProjectSettings {
        trust::for_reopen(
            job,
            self.shown.as_ref().map(|s| &s.observation),
            self.policies.for_owner(job.owner()),
        )
    }

    /// Where to look for Main Clones and the user's Claude directory.
    fn launch_settings(&self) -> LaunchSettings {
        LaunchSettings::from_config(
            self.config.as_ref().ok(),
            self.default_clone_root.clone(),
            self.claude_dir.clone(),
        )
    }

    /// The current Decision for `item`, if it is still surveyed.
    pub(super) fn decision_of(&self, item: &WorkItemId) -> Option<&Decision> {
        self.ordered.iter().find(|d| &d.item.id == item)
    }

    /// Plans the sheet's launch from the current Observation and `view`, asking
    /// `reviewers` for a Request review (`None`: the Decision's suggestions).
    fn replan(
        &self,
        sheet: &Sheet,
        view: &WorkspaceView,
        reviewers: Option<&[Reviewer]>,
    ) -> Result<LaunchPlan, Refusal> {
        let gone = || Refusal::one(handoff::RefusalReason::WorkItemGone);
        let decision = self.decision_of(&sheet.item).ok_or_else(gone)?;
        let observation = self
            .shown
            .as_ref()
            .map(|s| &s.observation)
            .ok_or_else(gone)?;
        let workflow = self.linear_workflow_of(&sheet.item);
        let slack_channel = workflow.and_then(|workflow| workflow.slack_channel.as_deref());
        let plan = handoff::plan(&PlanInput {
            decision,
            observation,
            choice: sheet.choice,
            view,
            prompts: self.prompts.as_ref(),
            fresh: sheet.fresh.as_ref().map(|(n, _)| *n),
            reviewers,
            slack_channel,
            linear_repo: sheet.linear.as_ref().and_then(LinearLaunch::repo),
            policy: self.policies.for_owner(sheet.item.owner()),
            removed_env: &self.removed_env,
            legacy_marker_prefixes: self.policies.legacy_marker_prefixes(),
        })?;
        Ok(match &sheet.accepted_head {
            Some(commit) => plan.on_commit(commit),
            None => plan,
        })
    }

    pub(super) fn launch_inspected(
        &mut self,
        id: LaunchId,
        view: Box<WorkspaceView>,
    ) -> Vec<Effect> {
        let Some(mut sheet) = self.current_sheet(id) else {
            return vec![stale(id, "inspection")];
        };
        self.remember_checks(&sheet.item, &view.job_checks);
        sheet.workflows = Some(view.workflows.clone());
        let existing = live::most_telling(&view.job_checks, &sheet.item)
            .filter(|_| sheet.fresh.is_none())
            .cloned();
        let effects = match (existing, sheet.launch_refused.clone()) {
            // A Live Job: ask, never launch a second one silently.
            (Some(check), _) if check.is_live() => {
                let log = asking_log(id, &sheet.item, "a Live Job", &check);
                sheet.state = SheetState::LiveJob {
                    check: Box::new(check),
                    view,
                };
                vec![log]
            }
            // Opened only to reopen, and the Job is no longer live: the fresh
            // launch's refusal stands.
            (_, Some(refusal)) => {
                let log = refused_log(id, &sheet.item, &refusal);
                sheet.state = SheetState::Refused(refusal);
                vec![log]
            }
            // Unknown: explain; never treated as gone, never relaunched silently.
            (Some(check), None) if matches!(check.state, WorktreeState::Unknown(_)) => {
                let log = asking_log(
                    id,
                    &sheet.item,
                    "a Job whose worktree den can't tell about",
                    &check,
                );
                sheet.state = SheetState::JobUnknown {
                    check: Box::new(check),
                    view,
                };
                vec![log]
            }
            // No Job, or its worktree is gone: launch as usual.
            _ => {
                let decision = self.decision_of(&sheet.item);
                sync_reviewers(&mut sheet, decision);
                let planned = planned_reviewers(&sheet, None);
                match self.replan(&sheet, &view, planned.as_deref()) {
                    Ok(plan) => {
                        show_plan(&mut sheet, plan, view, None);
                        Vec::new()
                    }
                    Err(refusal) => refuse(&mut sheet, refusal),
                }
            }
        };
        self.overlay = Some(OverlayState::Launch(sheet));
        effects
    }

    pub(super) fn launch_finished(
        &mut self,
        id: LaunchId,
        report: Box<LaunchReport>,
    ) -> Vec<Effect> {
        let mut effects = vec![finished_log(id, &report)];
        // A new Job was recorded: check the Jobs again so the list marks it.
        if matches!(report.outcome, LaunchOutcome::Launched { .. }) {
            effects.push(Effect::CheckJobs);
        }
        let Some(mut sheet) = self.current_sheet(id) else {
            // The sheet is gone but the launch happened: the log says so.
            return effects;
        };
        let (plan, view) = match sheet.state.clone() {
            SheetState::Launching { plan, view } => (plan, view),
            SheetState::Reopening { plan } => {
                sheet.state = SheetState::Reopened { plan, report };
                self.overlay = Some(OverlayState::Launch(sheet));
                return effects;
            }
            _ => {
                self.overlay = Some(OverlayState::Launch(sheet));
                return effects;
            }
        };
        sheet.state = match &report.outcome {
            LaunchOutcome::HeadMoved { from, to } => {
                sheet.accepted_head = Some(to.clone());
                SheetState::Ready {
                    plan: Box::new(plan.on_commit(to)),
                    view,
                    moved_from: Some(from.clone()),
                }
            }
            _ => SheetState::Finished { plan, report },
        };
        self.overlay = Some(OverlayState::Launch(sheet));
        effects
    }

    pub(super) fn launch_not_queued(&mut self, id: LaunchId, error: String) {
        if let Some(mut sheet) = self.current_sheet(id) {
            sheet.state =
                SheetState::Refused(Refusal::one(RefusalReason::RequestNotQueued { error }));
            self.overlay = Some(OverlayState::Launch(sheet));
        }
    }

    /// Keeps the fresher checks of `item`'s Jobs from a sheet's inspection.
    fn remember_checks(&mut self, item: &WorkItemId, checks: &[JobCheck]) {
        self.job_checks.retain(|c| !c.job.subject.covers(item));
        self.job_checks.extend(checks.iter().cloned());
    }

    /// Takes the open sheet out of the overlay when it is sheet `id`.
    fn current_sheet(&mut self, id: LaunchId) -> Option<Box<Sheet>> {
        match self.overlay.take() {
            Some(OverlayState::Launch(sheet)) if sheet.id == id => Some(sheet),
            other => {
                self.overlay = other;
                None
            }
        }
    }
}

/// The target the fork's own setting (the sidebar's `u`) names.
fn fork_setting(fork: Option<&ForkTargets>) -> TargetChoice {
    match fork {
        Some(fork) if fork.set_to_upstream => TargetChoice::Upstream,
        _ => TargetChoice::OwnRepo,
    }
}

/// `m` on a Linear Issue's sheet: the state move on or off.
fn toggle_move(sheet: &mut Sheet) {
    let Some(linear) = sheet.linear.as_mut() else {
        sheet.notice =
            Some("m does nothing here: only a Linear Issue's sheet moves its state".to_owned());
        return;
    };
    if linear.writes.finished {
        sheet.notice = Some("m does nothing now: the Linear writes are done".to_owned());
        return;
    }
    match &linear.writes.move_to {
        Some(_) => linear.writes.move_on = !linear.writes.move_on,
        None => {
            sheet.notice = Some(
                match &linear.in_progress_state {
                    None => "m does nothing here: the Linear Workflow names no in_progress_state",
                    Some(_) => {
                        "m does nothing here: the issue has already started, so den doesn't move it"
                    }
                }
                .to_owned(),
            );
        }
    }
}

/// Keeps a sheet's reviewers in step with the current Decision, before the
/// sheet plans with them: a Request review gets the field (a survey may have
/// made the Next Step Request review since the sheet opened), any other Next
/// Step loses it. The suggestions follow the Decision, and so does a list the
/// user never edited (it still reads as the old suggestions): it would
/// otherwise ask someone who has since approved, been asked or become the
/// author. A list the user edited is theirs and stays.
fn sync_reviewers(sheet: &mut Sheet, decision: Option<&Decision>) {
    let Some(decision) = decision.filter(|d| d.next_step == Some(NextStep::RequestReview)) else {
        sheet.reviewers = None;
        return;
    };
    let suggested = decision.suggested_reviewers.clone().unwrap_or_default();
    match sheet.reviewers.as_mut() {
        Some(field) => {
            if field.follows_suggestions() {
                // A target switch may still hold prompt edits for the old list.
                if field.suggested != suggested && sheet.edited_prompt.take().is_some() {
                    sheet.notice = Some(
                        "The reviewers changed, so den rebuilt the prompt from its template; your edits to the prompt were replaced".to_owned(),
                    );
                }
                field.editor = PromptEditor::new(&reviewers::spell_list(&suggested));
            }
            field.suggested = suggested;
            field.suggested_unread = decision.suggested_reviewers_unread.clone();
        }
        None => sheet.reviewers = Some(ReviewersField::new(decision)),
    }
}

/// What changed between the plan the sheet shows (with `shown_default`, the
/// prompt den built for it) and `now`, in what decides the Job; `None` when
/// nothing did.
fn what_changed(
    shown: &LaunchPlan,
    shown_default: Option<&str>,
    now: &LaunchPlan,
) -> Option<String> {
    if shown.next_step != now.next_step {
        return Some(format!(
            "the Next Step is now {} (was {})",
            now.next_step.label(),
            shown.next_step.label()
        ));
    }
    // The mode follows the Next Step and the Owner's Linear Workflow policy
    // (`job_permission_mode`, #41); the policy comes from the config, so in
    // practice only the Next Step moves it.
    if shown.permission_mode != now.permission_mode {
        return Some("the permission mode changed".to_owned());
    }
    // The viewer or the PR's author can differ in a newer survey.
    if shown.project_settings != now.project_settings {
        return Some("whether the repo's settings load changed".to_owned());
    }
    match (&shown.review_request, &now.review_request) {
        (Some(before), Some(after)) if before.reviewers != after.reviewers => {
            return Some("the suggested reviewers changed".to_owned());
        }
        (before, after) if before != after => {
            return Some("the announcement channel changed".to_owned());
        }
        _ => {}
    }
    // Land (#40): the members, their heads and the held members are what
    // the user approved; any difference asks again.
    match (&shown.landing, &now.landing) {
        (Some(before), Some(after)) => {
            if let Some(change) = after.change_from(before) {
                return Some(change);
            }
        }
        (before, after) if before != after => {
            return Some("the Landing Group changed".to_owned());
        }
        _ => {}
    }
    // Merge and Land (#78): the resolution comments are published as the
    // sheet showed them, so a Point that changed asks again. After the
    // Landing Group, whose changes change them too and say more.
    if shown.resolutions != now.resolutions {
        return Some("the resolution comment changed".to_owned());
    }
    shown_default
        .is_some_and(|default| default != now.prompt)
        .then(|| "the prompt den builds changed".to_owned())
}

/// Shows `plan` instead of the plan the sheet showed, because of `change`:
/// nothing is launched, and `Enter` launches only once the user has seen it.
/// A prompt the user edited was for the old plan, so den's new one replaces
/// it, and the sheet says so.
fn ask_again(
    sheet: &mut Sheet,
    plan: LaunchPlan,
    view: Box<WorkspaceView>,
    moved_from: Option<String>,
    change: &str,
) -> Vec<Effect> {
    let edited = sheet
        .default_prompt
        .as_deref()
        .is_some_and(|default| sheet.prompt.text() != default);
    let replaced = if edited {
        "; your edits to the prompt were replaced"
    } else {
        ""
    };
    sheet.notice = Some(format!(
        "Not launched: {change} since the sheet showed it. den planned again{replaced}; check the sheet, then press Enter to launch"
    ));
    let log = Effect::log(
        LogLevel::Info,
        format!(
            "launch sheet {} not launched for {}: {change}; asking again",
            sheet.id.0, sheet.item
        ),
    );
    sheet.edited_prompt = None;
    show_plan(sheet, plan, view, moved_from);
    vec![log]
}

/// Puts `plan` on the sheet, ready: its Next Step, its prompt (or the edit
/// kept across a `u`, [`Sheet::edited_prompt`]) and the default it was built
/// from. The one way a sheet adopts a plan, so they stay in step.
fn show_plan(
    sheet: &mut Sheet,
    plan: LaunchPlan,
    view: Box<WorkspaceView>,
    moved_from: Option<String>,
) {
    sheet.next_step = Some(plan.next_step);
    sheet.prompt = PromptEditor::new(sheet.edited_prompt.as_deref().unwrap_or(&plan.prompt));
    sheet.default_prompt = Some(plan.prompt.clone());
    sheet.state = SheetState::Ready {
        plan: Box::new(plan),
        view,
        moved_from,
    };
}

/// The reviewers a sheet plans with: its list when it parses. A list that
/// doesn't parse plans with what it stands for until `Enter` explains it: the
/// suggestions while it still reads as them (`None`), else the reviewers of
/// the plan the sheet shows (`shown`), so a survey's new suggestions never
/// read as a change the user made.
fn planned_reviewers(sheet: &Sheet, shown: Option<&LaunchPlan>) -> Option<Vec<Reviewer>> {
    let field = sheet.reviewers.as_ref()?;
    match field.chosen() {
        Ok(chosen) => Some(chosen),
        Err(_) if field.follows_suggestions() => None,
        Err(_) => shown
            .and_then(|plan| plan.review_request.as_ref())
            .map(|request| request.reviewers.clone()),
    }
}

/// The sheet refuses with `refusal`, logged.
fn refuse(sheet: &mut Sheet, refusal: Refusal) -> Vec<Effect> {
    let log = refused_log(sheet.id, &sheet.item, &refusal);
    sheet.state = SheetState::Refused(refusal);
    vec![log]
}

/// The inspection for reopening `item`'s Live Job alone, when den last saw
/// one (`checks`); `None` when it has none to reopen.
fn live_job_request(
    checks: &[JobCheck],
    item: &WorkItemId,
    observation: &Observation,
) -> Option<InspectRequest> {
    let check = live::most_telling(checks, item).filter(|c| c.is_live())?;
    InspectRequest::for_job(&check.job, item, observation)
}

/// `o` on a Live Job: plans the reopen, or refuses (never in the Main Clone).
/// `policy` is the Job's Owner's Linear Workflow policy.
fn open_existing(
    sheet: &mut Sheet,
    check: &JobCheck,
    view: &WorkspaceView,
    policy: &LinearWorkflowPolicy,
    settings: ProjectSettings,
    removed_env: &[String],
) -> Vec<Effect> {
    match live::plan_reopen(check, view, policy, settings, removed_env) {
        Ok(plan) => {
            sheet.state = SheetState::ReopenReady {
                plan: Box::new(plan),
            };
            Vec::new()
        }
        Err(refusal) => {
            let log = refused_log(sheet.id, &sheet.item, &refusal);
            sheet.state = SheetState::Refused(refusal);
            vec![log]
        }
    }
}

fn asking_log(id: LaunchId, item: &WorkItemId, what: &str, check: &JobCheck) -> Effect {
    Effect::log(
        LogLevel::Info,
        format!(
            "launch sheet {} for {item} found {what} at {}; asking",
            id.0,
            check.job.worktree.display()
        ),
    )
}

fn stale(id: LaunchId, what: &str) -> Effect {
    Effect::log(
        LogLevel::Info,
        format!(
            "ignored the {what} for launch sheet {}, which is closed",
            id.0
        ),
    )
}

fn refused_log(id: LaunchId, item: &WorkItemId, refusal: &Refusal) -> Effect {
    Effect::log(
        LogLevel::Warn,
        format!("launch sheet {} for {item} refused: {refusal}", id.0),
    )
}

fn finished_log(id: LaunchId, report: &LaunchReport) -> Effect {
    let done = report.done.join("; ");
    let (level, outcome) = match &report.outcome {
        LaunchOutcome::Launched { job } => (
            LogLevel::Info,
            format!(
                "launched {} in {} (session {})",
                job.subject,
                job.worktree.display(),
                job.session
            ),
        ),
        LaunchOutcome::Reopened { job } => (
            LogLevel::Info,
            format!(
                "reopened {} in {} (resuming session {})",
                job.subject,
                job.worktree.display(),
                job.session
            ),
        ),
        LaunchOutcome::HeadMoved { from, to } => (
            LogLevel::Warn,
            format!(
                "the base moved from {} to {}; asking again",
                short_commit(from),
                short_commit(to)
            ),
        ),
        LaunchOutcome::Refused(refusal) => (LogLevel::Warn, format!("refused: {refusal}")),
        LaunchOutcome::Failed { step, error } => {
            (LogLevel::Error, format!("could not {step}: {error}"))
        }
    };
    Effect::log(level, format!("launch {}: {outcome}; done: [{done}]", id.0))
}
