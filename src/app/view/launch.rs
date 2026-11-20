//! The launch sheet's ViewModel: what launching will do, then what it did.

use super::super::launch::{Sheet, SheetState};
use crate::handoff::{Target, TargetChoice};
use crate::{
    app::view_model::{LaunchSheet, LaunchStatus, PromptField, SheetField, SheetMessage, Tone},
    handoff::{
        BaseRef, BranchStart, CloneSource, LaunchOutcome, LaunchPlan, LaunchReport, PathState,
        Refusal, WindowMode, Writability, command, short_commit,
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
            view.fields.extend(chosen_target_field(sheet));
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
        }
        SheetState::Refused(refusal) => {
            view.status = LaunchStatus::Refused;
            view.headline = "Refused: nothing was done".to_owned();
            view.fields.extend(chosen_target_field(sheet));
            view.messages.extend(refusal_messages(refusal));
        }
        SheetState::Launching { plan, .. } => {
            view.status = LaunchStatus::Launching;
            view.headline = "Launching…".to_owned();
            fill_plan(&mut view, sheet, plan, None);
        }
        SheetState::Finished { plan, report } => {
            fill_plan(&mut view, sheet, plan, None);
            finished(&mut view, plan, report);
        }
    }
    if let Some(notice) = &sheet.notice {
        view.messages.push(message(notice, Tone::Caution));
    }
    view
}

fn fill_plan(view: &mut LaunchSheet, sheet: &Sheet, plan: &LaunchPlan, moved_from: Option<&str>) {
    // The plan's Next Step, not the one seen when the sheet opened: a survey
    // that finished in between may have changed it, and the plan is what runs.
    view.next_step = plan.next_step.label().to_owned();
    view.fields = fields(sheet, plan, moved_from);
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

/// The "upstream →" label: an Upstream target reads as one in monochrome too.
pub const UPSTREAM_LABEL: &str = "upstream →";

/// The TARGET field of a plan: the Upstream in rust with its text label, or the
/// Work Item's own Repo (a fork's own, by default).
fn target_field(sheet: &Sheet, target: &Target) -> SheetField {
    match &target.upstream_of {
        Some(fork) => field(
            "TARGET",
            format!("{UPSTREAM_LABEL} {}", target.repo),
            Some(upstream_note(sheet, fork)),
            Tone::Upstream,
        ),
        None => field(
            "TARGET",
            target.repo.to_string(),
            own_repo_note(sheet, target.is_fork),
            Tone::Plain,
        ),
    }
}

fn upstream_note(sheet: &Sheet, fork: &crate::domain::RepoId) -> String {
    if sheet.choice == sheet.fork_choice {
        format!("the Upstream of {fork}, as set for the fork (u: the fork, for this Job)")
    } else {
        format!("the Upstream of {fork}, for this Job only (u: back to the fork)")
    }
}

fn own_repo_note(sheet: &Sheet, is_fork: bool) -> Option<String> {
    if !is_fork {
        return None;
    }
    let mut note = "fork: its own Repo, not its Upstream".to_owned();
    if sheet.choice != sheet.fork_choice {
        note.push_str(", for this Job only");
    }
    Some(note)
}

/// The TARGET field before there is a plan (looking, or refused), from the
/// sheet's choice, so a refused Upstream target still shows what was chosen.
fn chosen_target_field(sheet: &Sheet) -> Option<SheetField> {
    let fork = sheet.fork.as_ref()?;
    Some(match (sheet.choice, &fork.upstream) {
        (TargetChoice::Upstream, Some(upstream)) => field(
            "TARGET",
            format!("{UPSTREAM_LABEL} {upstream}"),
            Some(upstream_note(sheet, &fork.fork)),
            Tone::Upstream,
        ),
        (TargetChoice::Upstream, None) => field(
            "TARGET",
            format!("{UPSTREAM_LABEL} (not visible)"),
            Some(format!("the Upstream of {}", fork.fork)),
            Tone::Upstream,
        ),
        (TargetChoice::OwnRepo, _) => field(
            "TARGET",
            fork.fork.to_string(),
            own_repo_note(sheet, true),
            Tone::Plain,
        ),
    })
}

fn fields(sheet: &Sheet, plan: &LaunchPlan, moved_from: Option<&str>) -> Vec<SheetField> {
    let mut fields = vec![target_field(sheet, &plan.target)];
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
    let (base, base_note) = match &plan.base.reference {
        BaseRef::HeadBranch(head) => (
            format!("PR head {head} @ {}", short_commit(&plan.base.commit)),
            None,
        ),
        BaseRef::PullHead { number } => (
            format!(
                "PR ref refs/pull/{number}/head @ {}",
                short_commit(&plan.base.commit)
            ),
            Some(format!(
                "the head branch lives in another Repo; fetched from {}, the target's remote",
                plan.remote.name
            )),
        ),
    };
    fields.push(match moved_from {
        Some(from) => field(
            "BASE",
            base,
            Some(format!("moved from {}", short_commit(from))),
            Tone::Caution,
        ),
        None => field("BASE", base, base_note, Tone::Plain),
    });
    fields.push(field(
        "BRANCH",
        plan.branch.name.clone(),
        Some(match &plan.branch.start {
            BranchStart::New { tracking } => format!("new, tracking {tracking}"),
            BranchStart::NewUntracked => {
                "new, tracking nothing (its head lives in another Repo)".to_owned()
            }
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
    fields.push(match &plan.window {
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
    });
    fields
}

fn worktree_note(state: &PathState) -> String {
    match state {
        PathState::Missing => "free".to_owned(),
        PathState::Exists { .. } => "already exists".to_owned(),
        PathState::Unknown(why) => format!("unknown: {why}"),
    }
}

fn finished(view: &mut LaunchSheet, plan: &LaunchPlan, report: &LaunchReport) {
    view.messages
        .extend(report.done.iter().map(|d| message(d, Tone::Good)));
    match &report.outcome {
        LaunchOutcome::Launched { .. } => {
            view.status = LaunchStatus::Launched;
            view.headline = "Launched".to_owned();
            view.messages.push(match &plan.window {
                WindowMode::Kitty => message("The Job's window is open.", Tone::Plain),
                WindowMode::Clipboard { .. } => message(
                    "The command is on the clipboard: paste it into a terminal.",
                    Tone::Plain,
                ),
            });
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
