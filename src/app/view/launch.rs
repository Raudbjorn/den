//! The launch sheet's ViewModel: what launching will do, then what it did.

use super::super::launch::{Sheet, SheetState};
use crate::{
    app::view_model::{LaunchSheet, LaunchStatus, PromptField, SheetField, SheetMessage, Tone},
    handoff::{
        BaseRef, BranchStart, CloneSource, JobCheck, LaunchOutcome, LaunchPlan, LaunchReport,
        PathState, Refusal, ReopenPlan, WindowMode, WorktreeState, Writability, command,
        short_commit,
    },
};

const LABEL: &str = "L A U N C H   J O B";

pub fn build(sheet: &Sheet) -> LaunchSheet {
    let mut view = LaunchSheet {
        label: LABEL.to_owned(),
        reference: sheet.item.to_string(),
        title: sheet.title.clone(),
        next_step: sheet
            .next_step
            .map_or("none", |step| step.label())
            .to_owned(),
        status: LaunchStatus::Inspecting,
        headline: String::new(),
        fields: Vec::new(),
        prompt: None,
        command: None,
        messages: Vec::new(),
    };
    match &sheet.state {
        SheetState::Inspecting => {
            view.status = LaunchStatus::Inspecting;
            view.headline = "Looking at the workspace…".to_owned();
        }
        SheetState::Ready {
            plan, moved_from, ..
        } => {
            view.status = match moved_from {
                Some(_) => LaunchStatus::HeadMoved,
                None => LaunchStatus::Ready,
            };
            view.headline = match moved_from {
                Some(from) => format!(
                    "The PR head moved from {} to {}: check, then ⏎ launches on {}",
                    short_commit(from),
                    short_commit(&plan.base.commit),
                    short_commit(&plan.base.commit)
                ),
                None => "Ready: ⏎ launches, esc cancels".to_owned(),
            };
            fill_plan(&mut view, sheet, plan, moved_from.as_deref());
            if let Some((_, beside)) = &sheet.fresh {
                view.messages.push(message(
                    &format!(
                        "Starting fresh: the Live Job's worktree {} is left as it is.",
                        beside.job.worktree.display()
                    ),
                    Tone::Plain,
                ));
            }
        }
        SheetState::Refused(refusal) => {
            view.status = LaunchStatus::Refused;
            view.headline = "Refused: nothing was done".to_owned();
            view.messages.extend(refusal_messages(refusal));
        }
        SheetState::Launching { plan, .. } => {
            view.status = LaunchStatus::Launching;
            view.headline = "Launching…".to_owned();
            fill_plan(&mut view, sheet, plan, None);
        }
        SheetState::Finished { plan, report } => {
            fill_plan(&mut view, sheet, plan, None);
            finished(&mut view, &plan.window, report);
        }
        SheetState::LiveJob { check, .. } => {
            view.status = LaunchStatus::LiveJob;
            view.headline = match &sheet.launch_refused {
                None => "This Work Item has a Live Job: open existing or start fresh?",
                Some(_) => "This Work Item has a Live Job: open existing?",
            }
            .to_owned();
            view.fields = job_fields(check);
            view.messages.push(message(
                &format!(
                    "o open existing: a new window in this worktree, resuming session {}",
                    check.job.session
                ),
                Tone::Plain,
            ));
            view.messages.push(match &sheet.launch_refused {
                None => message(
                    "f start fresh: a new worktree with a distinct name; this one is left as it is",
                    Tone::Plain,
                ),
                Some(refusal) => message(
                    &format!("Starting fresh isn't possible: {refusal}"),
                    Tone::Muted,
                ),
            });
        }
        SheetState::JobUnknown { check, .. } => {
            view.status = LaunchStatus::JobUnknown;
            view.headline =
                "den can't tell whether this Work Item's Job still has its worktree".to_owned();
            view.fields = job_fields(check);
            view.messages.push(message(
                "den won't treat it as gone, won't delete it, and won't launch again unless you ask: f starts fresh in a new worktree, esc cancels",
                Tone::Muted,
            ));
        }
        SheetState::ReopenReady { plan } => {
            view.status = LaunchStatus::ReopenReady;
            view.headline = "Ready: ⏎ opens the existing Job, esc cancels".to_owned();
            fill_reopen(&mut view, plan);
        }
        SheetState::Reopening { plan } => {
            view.status = LaunchStatus::Launching;
            view.headline = "Opening…".to_owned();
            fill_reopen(&mut view, plan);
        }
        SheetState::Reopened { plan, report } => {
            fill_reopen(&mut view, plan);
            view.messages.clear();
            finished(&mut view, &plan.window, report);
        }
    }
    if let Some(notice) = &sheet.notice {
        view.messages.push(message(notice, Tone::Caution));
    }
    view
}

fn fill_plan(view: &mut LaunchSheet, sheet: &Sheet, plan: &LaunchPlan, moved_from: Option<&str>) {
    view.fields = fields(plan, moved_from);
    let prompt = sheet.prompt.text();
    view.command = Some(command::display(
        &plan.window,
        &command::WindowCommand {
            prompt: prompt.clone(),
            ..plan.window_command()
        },
    ));
    view.prompt = Some(PromptField {
        text: prompt,
        cursor: sheet.prompt.cursor(),
        editing: sheet.editing,
    });
}

fn fields(plan: &LaunchPlan, moved_from: Option<&str>) -> Vec<SheetField> {
    let mut fields = vec![field(
        "TARGET",
        plan.target.repo.to_string(),
        plan.target
            .is_fork
            .then(|| "fork: its own Repo, not its Upstream".to_owned()),
        Tone::Plain,
    )];
    let (writable, tone) = match &plan.main_clone.writable {
        Writability::Writable => ("exists, writable".to_owned(), Tone::Good),
        Writability::NotWritable => ("exists, not writable".to_owned(), Tone::Caution),
        Writability::Unknown(why) => (format!("exists; writable unknown: {why}"), Tone::Caution),
    };
    let source = match &plan.main_clone.source {
        CloneSource::Configured => "set in [main_clones]".to_owned(),
        CloneSource::Searched { root, others } if others.is_empty() => {
            format!("found under {} by its remote", root.display())
        }
        CloneSource::Searched { root, others } => format!(
            "found under {} by its remote; also matching: {}",
            root.display(),
            others
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    fields.push(field(
        "MAIN CLONE",
        plan.main_clone.path.display().to_string(),
        Some(format!("{writable}; {source}")),
        tone,
    ));
    fields.push(field(
        "REMOTE",
        format!("{} → {}", plan.remote.name, plan.remote.url),
        Some("its URL is the target Repo".to_owned()),
        Tone::Good,
    ));
    let BaseRef::HeadBranch(head) = &plan.base.reference;
    fields.push(match moved_from {
        Some(from) => field(
            "BASE",
            format!("PR head {head} @ {}", short_commit(&plan.base.commit)),
            Some(format!("moved from {}", short_commit(from))),
            Tone::Caution,
        ),
        None => field(
            "BASE",
            format!("PR head {head} @ {}", short_commit(&plan.base.commit)),
            None,
            Tone::Plain,
        ),
    });
    fields.push(field(
        "BRANCH",
        plan.branch.name.clone(),
        Some(match &plan.branch.start {
            BranchStart::New { tracking } => format!("new, tracking {tracking}"),
            BranchStart::Existing => "existing local branch, already at the PR head".to_owned(),
        }),
        Tone::Plain,
    ));
    fields.push(field(
        "WORKTREE",
        plan.worktree.display().to_string(),
        Some(worktree_note(&PathState::Missing)),
        Tone::Good,
    ));
    fields.push(field("SESSION", plan.session.clone(), None, Tone::Plain));
    fields.push(opens_field(&plan.window));
    fields
}

fn worktree_note(state: &PathState) -> String {
    match state {
        PathState::Missing => "free".to_owned(),
        PathState::Exists { .. } => "already exists".to_owned(),
        PathState::Unknown(why) => format!("unknown: {why}"),
    }
}

/// The recorded Job's worktree, branch and session, with its state.
fn job_fields(check: &JobCheck) -> Vec<SheetField> {
    let job = &check.job;
    let (note, tone) = match &check.state {
        WorktreeState::Present => (
            "present: it exists and git has it registered".to_owned(),
            Tone::Good,
        ),
        WorktreeState::Missing => ("missing: its folder is gone".to_owned(), Tone::Muted),
        WorktreeState::Unknown(why) => (format!("unknown: {why}"), Tone::Caution),
    };
    vec![
        field("JOB", job.worktree.display().to_string(), Some(note), tone),
        field("BRANCH", job.branch.clone(), None, Tone::Plain),
        field(
            "SESSION",
            job.session.clone(),
            Some(format!(
                "launched {}",
                job.launched_at.strftime("%Y-%m-%d %H:%M UTC")
            )),
            Tone::Plain,
        ),
    ]
}

/// Open existing: the worktree, the resumed session, the window, the command,
/// and the duplicate-writer warning.
fn fill_reopen(view: &mut LaunchSheet, plan: &ReopenPlan) {
    let job = &plan.job;
    view.fields = vec![
        field(
            "WORKTREE",
            job.worktree.display().to_string(),
            Some("existing: nothing is fetched or added".to_owned()),
            Tone::Good,
        ),
        field("BRANCH", job.branch.clone(), None, Tone::Plain),
        field(
            "SESSION",
            job.session.clone(),
            Some(format!(
                "resumed by name (launched {})",
                job.launched_at.strftime("%Y-%m-%d %H:%M UTC")
            )),
            Tone::Plain,
        ),
        opens_field(&plan.window),
    ];
    view.command = Some(plan.command.clone());
    view.messages.push(message(
        "If the Job's original window is still open, both windows will write to the same conversation.",
        Tone::Caution,
    ));
}

fn opens_field(window: &WindowMode) -> SheetField {
    match window {
        WindowMode::Kitty => field(
            "OPENS",
            "a new kitty OS window".to_owned(),
            None,
            Tone::Plain,
        ),
        WindowMode::Clipboard { why } => field(
            "OPENS",
            "nothing: the command is copied to the clipboard".to_owned(),
            Some(why.clone()),
            Tone::Caution,
        ),
    }
}

fn finished(view: &mut LaunchSheet, window: &WindowMode, report: &LaunchReport) {
    view.messages
        .extend(report.done.iter().map(|d| message(d, Tone::Good)));
    let opened = |view: &mut LaunchSheet| {
        view.messages.push(match window {
            WindowMode::Kitty => message("The Job's window is open.", Tone::Plain),
            WindowMode::Clipboard { .. } => message(
                "The command is on the clipboard: paste it into a terminal.",
                Tone::Plain,
            ),
        });
    };
    match &report.outcome {
        LaunchOutcome::Launched { .. } => {
            view.status = LaunchStatus::Launched;
            view.headline = "Launched".to_owned();
            opened(view);
        }
        LaunchOutcome::Reopened { .. } => {
            view.status = LaunchStatus::Reopened;
            view.headline = "Reopened the existing Job".to_owned();
            opened(view);
        }
        LaunchOutcome::Refused(refusal) => {
            view.status = LaunchStatus::Refused;
            view.headline = "Refused just before launching: nothing was done".to_owned();
            view.messages.extend(refusal_messages(refusal));
        }
        LaunchOutcome::Failed { step, error } => {
            view.status = LaunchStatus::Failed;
            view.headline = format!("Stopped: could not {step}");
            view.messages.push(message(
                &format!("could not {step}: {error}"),
                Tone::Caution,
            ));
            view.messages.push(message(
                "Nothing was deleted; the steps above were left as they are.",
                Tone::Muted,
            ));
        }
        // The App turns a moved head back into a ready sheet; never shown here.
        LaunchOutcome::HeadMoved { .. } => {}
    }
}

fn refusal_messages(refusal: &Refusal) -> impl Iterator<Item = SheetMessage> + '_ {
    refusal
        .reasons
        .iter()
        .map(|reason| message(&reason.to_string(), Tone::Caution))
}

fn field(label: &str, value: String, note: Option<String>, tone: Tone) -> SheetField {
    SheetField {
        label: label.to_owned(),
        value,
        note,
        tone,
    }
}

fn message(text: &str, tone: Tone) -> SheetMessage {
    SheetMessage {
        text: text.to_owned(),
        tone,
    }
}
