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
//! Every ready sheet lists the slash commands its prompt invokes, as den
//! found them in the user's and the target Repo's Claude command and skill
//! directories, or "not found" with where den looked (#41). The inspection
//! reads the directories once; an edited prompt is checked against the same
//! read.
//!
//! `Enter` plans again from the current Decision. When a survey that landed
//! while the sheet was open changed what decides the Job (its Next Step, and
//! so its permission mode and prompt template; its reviewers; den's prompt),
//! nothing is launched: the sheet shows the new plan, says what changed, and
//! asks again (spec 0001, "Re-verification").

use super::{
    App, Effect, Key, LogLevel, OverlayState,
    editor::{Edit, PromptEditor},
    event::LaunchId,
};
use crate::{
    decide::Decision,
    domain::{LinearWorkflowPolicy, NextStep, Observation, RepoId, WorkItemId, facts::Reviewer},
    handoff::{
        self, InspectRequest, JobCheck, LaunchOutcome, LaunchPlan, LaunchReport, LaunchSettings,
        PlanInput, Refusal, RefusalReason, ReopenPlan, TargetChoice, WorkspaceView, WorktreeState,
        check_prompt, live, names, reviewers, short_commit, workflows::WorkflowDirs,
    },
};

/// One open launch sheet.
#[derive(Debug, Clone)]
pub(super) struct Sheet {
    pub id: LaunchId,
    pub item: WorkItemId,
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
    /// The Claude directories the last inspection read (#41): the sheet
    /// checks whatever prompt it shows, edited or not, against them.
    pub workflows: Option<WorkflowDirs>,
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
    Inspecting,
    Ready {
        plan: Box<LaunchPlan>,
        view: Box<WorkspaceView>,
        /// The commit the head moved from, when it moved since the sheet opened.
        moved_from: Option<String>,
    },
    Refused(Refusal),
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
        let id = LaunchId(self.next_launch);
        self.next_launch += 1;
        let fork = self.shown.as_ref().and_then(|shown| {
            let repo = shown.observation.repo(decision.item.id.repo()?)?;
            repo.is_fork.then(|| ForkTargets {
                fork: repo.id.clone(),
                upstream: repo.upstream().map(|u| u.id.clone()),
                set_to_upstream: self.upstreams.contains(&repo.id),
            })
        });
        let fork_setting = match &fork {
            Some(fork) if fork.set_to_upstream => TargetChoice::Upstream,
            _ => TargetChoice::OwnRepo,
        };
        // A PR's or Branch's Job starts on its own Repo whatever the fork's
        // setting; Issues and Runs follow it.
        let fork_choice = TargetChoice::default_for(&decision.item.id, fork_setting);
        let mut sheet = Sheet {
            id,
            item: decision.item.id.clone(),
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
            workflows: None,
        };
        let effects = self.inspect_for(&mut sheet, "opened");
        self.overlay = Some(OverlayState::Launch(Box::new(sheet)));
        effects
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
        let request = match handoff::inspect_request(decision, observation, sheet.choice, &settings)
        {
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
            sheet.notice = Some(format!(
                "u does nothing here: {} is not a fork, so it has no Upstream to target",
                sheet
                    .item
                    .repo()
                    .map_or_else(|| sheet.item.to_string(), ToString::to_string)
            ));
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
            (Key::Esc, _)
            | (
                Key::Enter,
                SheetState::Refused(_) | SheetState::Finished { .. } | SheetState::Reopened { .. },
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
                let policy = self.policies.for_owner(check.job.owner());
                effects = open_existing(&mut sheet, &check, &view, policy);
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

    /// `Enter` on a ready sheet: checks the prompt, plans again against what den
    /// knows now (a refresh may have archived the Repo), and launches. When a
    /// survey that landed while the sheet was open changed what decides the
    /// Job (its Next Step, and so its permission mode and prompt template; its
    /// reviewers; its default prompt), den launches nothing: the sheet shows
    /// the new plan and `Enter` asks again.
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
        let text = sheet.prompt.text();
        if let Err(problem) = check_prompt(&text) {
            sheet.notice = Some(format!("{problem}; edit it with e"));
            return Vec::new();
        }
        let decision = self.decision_of(&sheet.item);
        sync_reviewers(sheet, decision);
        if let Some(Err(problem)) = sheet.reviewers.as_ref().map(ReviewersField::chosen) {
            sheet.notice = Some(format!("{problem}; edit the reviewers with r"));
            return Vec::new();
        }
        match self.replan(sheet, &view) {
            Ok(plan) => {
                if let Some(change) = what_changed(&shown, sheet.default_prompt.as_deref(), &plan) {
                    return ask_again(sheet, plan, view, moved_from, &change);
                }
                let plan = match plan.with_prompt(&text) {
                    Ok(plan) => plan,
                    Err(problem) => {
                        sheet.notice = Some(format!("{problem}; edit it with e"));
                        return Vec::new();
                    }
                };
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
            Err(refusal) => {
                let log = refused_log(sheet.id, &sheet.item, &refusal);
                sheet.state = SheetState::Refused(refusal);
                vec![log]
            }
        }
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
                handoff::inspect_request(decision, observation, sheet.choice, &settings)
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
    /// explained and changes nothing else.
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
            plan,
            view,
            moved_from,
        } = &sheet.state
        else {
            return Vec::new();
        };
        if plan
            .review_request
            .as_ref()
            .is_some_and(|r| r.reviewers == chosen)
        {
            return Vec::new();
        }
        let (view, moved_from) = (view.clone(), moved_from.clone());
        match self.replan(sheet, &view) {
            Ok(plan) => {
                let default = sheet.default_prompt.as_deref().unwrap_or(&plan.prompt);
                if sheet.prompt.text() != default {
                    sheet.notice = Some(
                        "The reviewers changed, so den rebuilt the prompt from its template; your edits to the prompt were replaced".to_owned(),
                    );
                }
                sheet.prompt = PromptEditor::new(&plan.prompt);
                sheet.default_prompt = Some(plan.prompt.clone());
                sheet.edited_prompt = None;
                sheet.state = SheetState::Ready {
                    plan: Box::new(plan),
                    view,
                    moved_from,
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

    /// Where to look for Main Clones and the user's Claude directory.
    fn launch_settings(&self) -> LaunchSettings {
        LaunchSettings::from_config(
            self.config.as_ref().ok(),
            self.default_clone_root.clone(),
            self.claude_dir.clone(),
        )
    }

    /// The current Decision for `item`, if it is still surveyed.
    fn decision_of(&self, item: &WorkItemId) -> Option<&Decision> {
        self.ordered.iter().find(|d| &d.item.id == item)
    }

    /// Brings the sheet's reviewers in step with the current Decision, then
    /// plans with them.
    fn synced_plan(&self, sheet: &mut Sheet, view: &WorkspaceView) -> Result<LaunchPlan, Refusal> {
        let decision = self.decision_of(&sheet.item);
        sync_reviewers(sheet, decision);
        self.replan(sheet, view)
    }

    /// Plans the sheet's launch from the current Observation and `view`.
    fn replan(&self, sheet: &Sheet, view: &WorkspaceView) -> Result<LaunchPlan, Refusal> {
        let gone = || Refusal::one(handoff::RefusalReason::WorkItemGone);
        let decision = self.decision_of(&sheet.item).ok_or_else(gone)?;
        let observation = self
            .shown
            .as_ref()
            .map(|s| &s.observation)
            .ok_or_else(gone)?;
        // A list that doesn't parse plans with the suggested reviewers; `Enter`
        // refuses to launch until it parses.
        let chosen = sheet
            .reviewers
            .as_ref()
            .and_then(|field| field.chosen().ok());
        let slack_channel = sheet
            .item
            .repo()
            .and_then(|repo| self.config.as_ref().ok()?.linear_workflow(&repo.owner))
            .and_then(|workflow| workflow.slack_channel.as_deref());
        let plan = handoff::plan(&PlanInput {
            decision,
            observation,
            choice: sheet.choice,
            view,
            prompts: self.prompts.as_ref(),
            fresh: sheet.fresh.as_ref().map(|(n, _)| *n),
            reviewers: chosen.as_deref(),
            slack_channel,
            policy: self.policies.for_owner(sheet.item.owner()),
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
            _ => match self.synced_plan(&mut sheet, &view) {
                Ok(plan) => {
                    sheet.prompt =
                        PromptEditor::new(sheet.edited_prompt.as_deref().unwrap_or(&plan.prompt));
                    sheet.default_prompt = Some(plan.prompt.clone());
                    sheet.state = SheetState::Ready {
                        plan: Box::new(plan),
                        view,
                        moved_from: None,
                    };
                    Vec::new()
                }
                Err(refusal) => {
                    let log = refused_log(id, &sheet.item, &refusal);
                    sheet.state = SheetState::Refused(refusal);
                    vec![log]
                }
            },
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
            if field.editor.text() == reviewers::spell_list(&field.suggested) {
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
    if shown.permission_mode != now.permission_mode {
        return Some("the permission mode changed".to_owned());
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
    sheet.next_step = Some(plan.next_step);
    sheet.prompt = PromptEditor::new(&plan.prompt);
    sheet.default_prompt = Some(plan.prompt.clone());
    sheet.edited_prompt = None;
    sheet.state = SheetState::Ready {
        plan: Box::new(plan),
        view,
        moved_from,
    };
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
) -> Vec<Effect> {
    match live::plan_reopen(check, view, policy) {
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
