//! The launch sheet's ViewModel: what launching will do, then what it did.

use super::super::launch::{ReviewersField, Sheet, SheetState};
use crate::handoff::{Target, TargetChoice};
use crate::{
    app::view_model::{LaunchSheet, LaunchStatus, PromptField, SheetField, SheetMessage, Tone},
    domain::{WorkItemId, WorkItemKind},
    handoff::{
        BaseRef, BranchStart, CloneSource, JobCheck, LaunchOutcome, LaunchPlan, LaunchReport,
        PathState, PermissionMode, Refusal, ReopenPlan, ReviewRequest, WindowMode, WorktreeState,
        Writability, command, reviewers, short_commit, workflows,
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
        reviewers: None,
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
                    "{} moved from {} to {}: check, then ⏎ launches on {}",
                    moved_subject(plan),
                    short_commit(from),
                    short_commit(&plan.base.commit),
                    short_commit(&plan.base.commit)
                ),
                None if plan.clones() => format!(
                    "Ready: ⏎ clones {}, then launches; esc cancels",
                    plan.target.repo
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
    view.reviewers = plan
        .review_request
        .as_ref()
        .and(sheet.reviewers.as_ref())
        .map(reviewers_field);
    // The detail pane's caveat on the suggestions (#47), where they are used.
    if let Some(unread) = plan
        .review_request
        .as_ref()
        .and(sheet.reviewers.as_ref())
        .and_then(|r| r.suggested_unread.as_deref())
    {
        view.messages.push(message(
            &format!(
                "The suggested reviewers may include someone already asked or approved: {unread}"
            ),
            Tone::Caution,
        ));
    }
}

fn reviewers_field(field: &ReviewersField) -> PromptField {
    PromptField {
        text: field.editor.text(),
        cursor: field.editor.cursor(),
        editing: field.editing,
    }
}

/// A Request review's suggested reviewers and its Slack channel (#39).
fn review_request_fields(
    request: &ReviewRequest,
    suggested: &[crate::domain::facts::Reviewer],
) -> [SheetField; 2] {
    let suggested = field(
        "SUGGESTED",
        if suggested.is_empty() {
            "none".to_owned()
        } else {
            reviewers::spell_list(suggested)
        },
        Some(
            "configured reviewers and anyone awaiting re-review, minus fresh approvers, pending requests and the author; r edits whom the Job asks".to_owned(),
        ),
        Tone::Plain,
    );
    let announce = match &request.slack_channel {
        Some(channel) => field(
            "ANNOUNCE",
            channel.clone(),
            Some(
                "the Job posts it after requesting, best-effort; den itself never touches Slack"
                    .to_owned(),
            ),
            Tone::Plain,
        ),
        None => field(
            "ANNOUNCE",
            "nowhere".to_owned(),
            Some(
                "no Slack channel in this Owner's Linear Workflow: the prompt has no announcement step"
                    .to_owned(),
            ),
            Tone::Muted,
        ),
    };
    [suggested, announce]
}

/// The label of a slash command's availability (#41).
pub const WORKFLOW_LABEL: &str = "WORKFLOW";

/// One slash command the prompt invokes: where den found it, or "not found"
/// (amber, with the caution glyph) and where den looked (#41).
fn workflow_field(check: &workflows::WorkflowCheck) -> SheetField {
    if check.is_found() {
        field(
            WORKFLOW_LABEL,
            format!("{} found", check.command),
            Some(check.explanation()),
            Tone::Good,
        )
    } else {
        field(
            WORKFLOW_LABEL,
            format!("{} not found", check.command),
            Some(check.explanation()),
            Tone::Caution,
        )
    }
}

/// The PERMISSIONS field of a coordination Job (#39) or a verdict-only
/// Review Job (#41), or none for any other Job, whose command is milestone 1's.
fn permissions_field(mode: Option<PermissionMode>) -> Option<SheetField> {
    let mode = mode?;
    Some(field(
        "PERMISSIONS",
        format!("{}: Claude asks before acting", mode.label()),
        Some(format!(
            "{} (Claude Code's Manual mode); your allow rules and hooks still apply",
            mode.args().join(" ")
        )),
        Tone::Good,
    ))
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
    } else if sheet.fork.as_ref().is_some_and(|f| f.set_to_upstream) {
        // The fork targets its Upstream, but this kind's Job doesn't follow.
        let kind = match sheet.item {
            WorkItemId::Branch { .. } => "a Branch's Job targets the Repo the Branch is in",
            _ => "a PR's Job targets the Repo the PR is in",
        };
        note.push_str(&format!("; the fork targets its Upstream, but {kind}"));
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
    fields.push(main_clone_field(plan));
    fields.push(field(
        "REMOTE",
        format!("{} → {}", plan.remote.name, plan.remote.url),
        Some(if plan.clones() {
            "the clone's remote: its URL is the target Repo".to_owned()
        } else {
            "its URL is the target Repo".to_owned()
        }),
        Tone::Good,
    ));
    let base = format!("{} @ {}", base_words(plan), short_commit(&plan.base.commit));
    let base_note = matches!(plan.base.reference, BaseRef::PullHead { .. }).then(|| {
        format!(
            "the head branch lives in another Repo; fetched from {}, the target's remote",
            plan.remote.name
        )
    });
    fields.push(match moved_from {
        Some(from) => field(
            "BASE",
            base,
            Some(format!("moved from {}", short_commit(from))),
            Tone::Caution,
        ),
        None => field("BASE", base, base_note, Tone::Plain),
    });
    let mut branch_note = match &plan.branch.start {
        BranchStart::New { tracking } => format!("new, tracking {tracking}"),
        BranchStart::NewUntracked => {
            "new, tracking nothing (its head lives in another Repo)".to_owned()
        }
        BranchStart::Existing if plan.work_item.kind() == WorkItemKind::PullRequest => {
            "existing local branch, already at the PR head".to_owned()
        }
        BranchStart::Existing => "existing local branch, already at the base".to_owned(),
        BranchStart::Fresh => format!(
            "new, from {}; tracks nothing until pushed",
            plan.base
                .reference
                .branch()
                .map_or_else(|| base_words(plan), |b| format!("{}/{b}", plan.remote.name))
        ),
    };
    if let Some(from) = &plan.branch.renamed_from {
        branch_note.push_str(&format!(
            "; {from} is taken in the Main Clone, so den names it {}",
            plan.branch.name
        ));
    }
    fields.push(field(
        "BRANCH",
        plan.branch.name.clone(),
        Some(branch_note),
        Tone::Plain,
    ));
    fields.push(match &plan.renamed {
        None => field(
            "WORKTREE",
            plan.worktree.display().to_string(),
            Some(worktree_note(&PathState::Missing)),
            Tone::Good,
        ),
        Some(renamed) => field(
            "WORKTREE",
            plan.worktree.display().to_string(),
            Some(format!(
                "free; {} is taken by the Job for {}, so den added a number",
                renamed.wanted,
                renamed
                    .taken_by
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Tone::Caution,
        ),
    });
    fields.push(field("SESSION", plan.session.clone(), None, Tone::Plain));
    if let Some(request) = &plan.review_request {
        let suggested = sheet
            .reviewers
            .as_ref()
            .map_or(&[][..], |r| r.suggested.as_slice());
        fields.extend(review_request_fields(request, suggested));
    }
    fields.extend(permissions_field(plan.permission_mode));
    // The prompt as shown, edited or not, against what the inspection read.
    if let Some(dirs) = &sheet.workflows {
        fields.extend(
            workflows::check(&sheet.prompt.text(), dirs)
                .iter()
                .map(workflow_field),
        );
    }
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
    ];
    view.fields.extend(permissions_field(plan.permission_mode));
    view.fields.push(opens_field(&plan.window));
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

/// What the base is, in words: "PR head feature-7", "branch feature-7",
/// "default branch main".
fn base_words(plan: &LaunchPlan) -> String {
    match &plan.base.reference {
        BaseRef::HeadBranch(head) if plan.work_item.kind() == WorkItemKind::PullRequest => {
            format!("PR head {head}")
        }
        BaseRef::HeadBranch(branch) => format!("branch {branch}"),
        BaseRef::DefaultBranch(branch) => format!("default branch {branch}"),
        BaseRef::PullHead { number } => format!("PR ref refs/pull/{number}/head"),
    }
}

/// What moved, for the headline: "The PR head", "The default branch main".
fn moved_subject(plan: &LaunchPlan) -> String {
    if plan.work_item.kind() == WorkItemKind::PullRequest {
        "The PR head".to_owned()
    } else {
        format!("The {}", base_words(plan))
    }
}

fn main_clone_field(plan: &LaunchPlan) -> SheetField {
    let (writable, tone) = match &plan.main_clone.writable {
        Writability::Writable => ("exists, writable".to_owned(), Tone::Good),
        Writability::NotWritable => ("exists, not writable".to_owned(), Tone::Caution),
        Writability::Unknown(why) => (format!("exists; writable unknown: {why}"), Tone::Caution),
    };
    let source = match &plan.main_clone.source {
        CloneSource::ToClone { url, root, taken } => {
            let root_writable = match &plan.main_clone.writable {
                Writability::Writable => String::new(),
                Writability::NotWritable => format!(" ({} is not writable)", root.display()),
                Writability::Unknown(why) => {
                    format!(
                        " (whether den may write in {} is unknown: {why})",
                        root.display()
                    )
                }
            };
            let note = match taken.as_slice() {
                [] => format!("not cloned yet: launching clones {url} here{root_writable}"),
                taken => format!(
                    "not cloned yet: launching clones {url} here{root_writable}; {} is taken",
                    taken
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            return field(
                "MAIN CLONE",
                plan.main_clone.path.display().to_string(),
                Some(note),
                Tone::Caution,
            );
        }
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
    field(
        "MAIN CLONE",
        plan.main_clone.path.display().to_string(),
        Some(format!("{writable}; {source}")),
        tone,
    )
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
