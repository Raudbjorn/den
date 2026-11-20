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

use super::{
    App, Effect, Key, LogLevel, OverlayState,
    editor::{Edit, PromptEditor},
    event::LaunchId,
};
use crate::{
    domain::{NextStep, RepoId, WorkItemId},
    handoff::{
        self, LaunchOutcome, LaunchPlan, LaunchReport, LaunchSettings, PlanInput, Refusal,
        TargetChoice, WorkspaceView, check_prompt, short_commit,
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
    /// The fork's own setting when the sheet opened; `choice` differs from it
    /// only after `u` on the sheet (a one-Job override).
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
}

/// The two targets a Work Item in a fork can have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ForkTargets {
    pub fork: RepoId,
    pub upstream: Option<RepoId>,
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
            let repo = shown.observation.repo(decision.item.id.repo())?;
            repo.is_fork.then(|| ForkTargets {
                fork: repo.id.clone(),
                upstream: repo.upstream().map(|u| u.id.clone()),
            })
        });
        let fork_choice = match &fork {
            Some(fork) if self.upstreams.contains(&fork.fork) => TargetChoice::Upstream,
            _ => TargetChoice::OwnRepo,
        };
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
        };
        let effects = self.inspect_for(&mut sheet, "opened");
        self.overlay = Some(OverlayState::Launch(Box::new(sheet)));
        effects
    }

    /// Asks to inspect the workspace for the sheet's Work Item and target, or
    /// refuses straight away when den won't even look.
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
        let Some(decision) = self.ordered.iter().find(|d| d.item.id == sheet.item) else {
            return gone(sheet);
        };
        let settings =
            LaunchSettings::from_config(self.config.as_ref().ok(), self.default_clone_root.clone());
        match handoff::inspect_request(decision, observation, sheet.choice, &settings) {
            Ok(request) => {
                sheet.state = SheetState::Inspecting;
                vec![
                    Effect::log(
                        LogLevel::Info,
                        format!(
                            "launch sheet {} {what} for {}, targeting {}",
                            sheet.id.0, sheet.item, request.target
                        ),
                    ),
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
                sheet.item.repo()
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
            (Key::Esc, SheetState::Launching { .. }) => {
                sheet.notice = Some("den is launching; wait for it to finish".to_owned());
            }
            (Key::Esc, _) | (Key::Enter, SheetState::Refused(_) | SheetState::Finished { .. }) => {
                // Closed: nothing was or will be done by this sheet any more.
                return Vec::new();
            }
            (Key::Char('e') | Key::Tab, SheetState::Ready { .. }) => sheet.editing = true,
            (
                Key::Char('u'),
                SheetState::Ready { .. } | SheetState::Refused(_) | SheetState::Inspecting,
            ) => effects = self.switch_target(&mut sheet),
            (Key::Enter, SheetState::Ready { .. }) => effects = self.launch(&mut sheet),
            _ => {}
        }
        self.overlay = Some(OverlayState::Launch(sheet));
        effects
    }

    /// `Enter` on a ready sheet: checks the prompt, plans again against what den
    /// knows now (a refresh may have archived the Repo), and launches.
    fn launch(&self, sheet: &mut Sheet) -> Vec<Effect> {
        let SheetState::Ready { view, .. } = &sheet.state else {
            return Vec::new();
        };
        let text = sheet.prompt.text();
        if let Err(problem) = check_prompt(&text) {
            sheet.notice = Some(format!("{problem}; edit it with e"));
            return Vec::new();
        }
        let view = view.clone();
        match self.replan(sheet, &view) {
            Ok(plan) => {
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

    /// Plans the sheet's launch from the current Observation and `view`.
    fn replan(&self, sheet: &Sheet, view: &WorkspaceView) -> Result<LaunchPlan, Refusal> {
        let gone = || Refusal::one(handoff::RefusalReason::WorkItemGone);
        let decision = self
            .ordered
            .iter()
            .find(|d| d.item.id == sheet.item)
            .ok_or_else(gone)?;
        let observation = self
            .shown
            .as_ref()
            .map(|s| &s.observation)
            .ok_or_else(gone)?;
        let plan = handoff::plan(&PlanInput {
            decision,
            observation,
            choice: sheet.choice,
            view,
            prompts: self.prompts.as_ref(),
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
        let effects = match self.replan(&sheet, &view) {
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
        };
        self.overlay = Some(OverlayState::Launch(sheet));
        effects
    }

    pub(super) fn launch_finished(
        &mut self,
        id: LaunchId,
        report: Box<LaunchReport>,
    ) -> Vec<Effect> {
        let Some(mut sheet) = self.current_sheet(id) else {
            // The sheet is gone but the launch happened: say so in the log.
            return vec![finished_log(id, &report)];
        };
        let log = finished_log(id, &report);
        let SheetState::Launching { plan, view } = sheet.state.clone() else {
            self.overlay = Some(OverlayState::Launch(sheet));
            return vec![log];
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
        vec![log]
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
                job.work_item,
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
