//! The launch sheet's ViewModel: what launching will do, then what it did.

use super::super::{
    launch::{LinearLaunch, ReviewersField, Sheet, SheetState},
    linear_writes::{self, WriteResult, Writes},
    release::{ReleaseSheet, ReleaseState},
};
use crate::handoff::{Target, TargetChoice};
use crate::{
    app::view_model::{LaunchSheet, LaunchStatus, PromptField, SheetField, SheetMessage, Tone},
    domain::{WorkItemId, WorkItemKind},
    handoff::{
        BaseRef, BranchStart, CloneSource, JobCheck, LandingGroupPlan, LaunchOutcome, LaunchPlan,
        LaunchReport, PathState, PermissionMode, ProjectSettings, Refusal, ReopenPlan,
        ReviewRequest, WindowMode, WorktreeState, Writability, command, landing,
        resolutions::Resolution, reviewers, short_commit, workflows,
    },
};

const LABEL: &str = "L A U N C H   J O B";
/// The Mark for release sheet's label (#38).
const RELEASE_LABEL: &str = "M A R K   F O R   R E L E A S E";
/// Said on every sheet that writes to Linear: Linear has no compare-and-set.
const NO_COMPARE_AND_SET: &str = "den re-reads the issue just before writing, but Linear has no compare-and-set: someone may change it in between, and den shows what Linear returns";

pub fn build(sheet: &Sheet) -> LaunchSheet {
    let mut view = LaunchSheet {
        label: LABEL.to_owned(),
        reference: sheet.reference.clone(),
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
        linear_writes: Vec::new(),
        command: None,
        messages: Vec::new(),
    };
    match &sheet.state {
        SheetState::ChooseRepo => {
            view.status = LaunchStatus::ChooseRepo;
            view.headline = "Choose the target Repo: t picks the next one".to_owned();
        }
        SheetState::Writing { plan, .. } => {
            view.status = LaunchStatus::Writing;
            view.headline = "Writing to Linear before launching…".to_owned();
            fill_plan(&mut view, sheet, plan, None);
        }
        SheetState::Stopped { plan } => {
            fill_plan(&mut view, sheet, plan, None);
            view.status = LaunchStatus::Failed;
            view.headline = "Stopped: nothing launched".to_owned();
        }
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
            // A member's sheet names the Land Job that covers it (#40).
            let which = match check.job.subject.landing_group() {
                Some(group) if !group.issue.same(&sheet.item) => {
                    format!("is covered by the Live Land Job of {}", group.identifier)
                }
                _ => "has a Live Job".to_owned(),
            };
            view.headline = match &sheet.launch_refused {
                None => format!("This Work Item {which}: open existing or start fresh?"),
                Some(_) => format!("This Work Item {which}: open existing?"),
            };
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
    if let Some(linear) = &sheet.linear {
        add_linear(&mut view, sheet, linear);
    }
    if let Some(notice) = &sheet.notice {
        view.messages.push(message(notice, Tone::Caution));
    }
    view
}

/// A Linear Issue's Start or Implement sheet (#38): the issue, the Repos for
/// new work, the move toggle and Linear's branch name among the fields; the
/// writes as their own lines; what they did first among the messages.
fn add_linear(view: &mut LaunchSheet, sheet: &Sheet, linear: &LinearLaunch) {
    let writes = &linear.writes;
    let mut fields = vec![issue_field(writes)];
    if linear.repos.len() > 1 || linear.repo.is_none() {
        let offered = linear
            .repos
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        fields.push(match linear.repo() {
            Some(repo) => field(
                "REPO",
                repo.to_string(),
                Some(format!("chosen from {offered}; t picks the next")),
                Tone::Plain,
            ),
            None if linear.repos.is_empty() => field(
                "REPO",
                "none offered".to_owned(),
                Some("the Linear Workflow lists no new_work_repos".to_owned()),
                Tone::Caution,
            ),
            None => field(
                "REPO",
                "none chosen".to_owned(),
                Some(format!("offered for new work: {offered}; t picks one")),
                Tone::Caution,
            ),
        });
    }
    if !writes.finished {
        fields.push(move_field(sheet, linear));
    }
    for existing in &mut view.fields {
        if existing.label == "BRANCH" {
            let note = match &linear.suggested_branch {
                Some(name) if *name == existing.value => "Linear's suggested name".to_owned(),
                Some(name) => format!(
                    "Linear suggests {name}, which isn't a safe branch name, so den named it"
                ),
                None => "Linear suggests no name, so den named it".to_owned(),
            };
            existing.note = Some(match existing.note.take() {
                Some(before) => format!("{before}; {note}"),
                None => note,
            });
        }
    }
    fields.append(&mut view.fields);
    view.fields = fields;
    view.linear_writes = writes.planned();
    // The writes were made, then the launch didn't happen: never "nothing
    // was done".
    if writes.finished && matches!(view.status, LaunchStatus::Refused | LaunchStatus::Failed) {
        view.headline = format!(
            "{}; den's Linear writes above were made and stay",
            view.headline
                .replace(": nothing was done", ": nothing launched")
        );
    }
    let mut messages = result_messages(writes);
    if writes.pending() && matches!(sheet.state, SheetState::Ready { .. }) {
        messages.push(message(NO_COMPARE_AND_SET, Tone::Muted));
    }
    if matches!(sheet.state, SheetState::Stopped { .. }) {
        messages.push(message(
            "Nothing was undone: den never un-assigns an issue or moves it back.",
            Tone::Muted,
        ));
    }
    messages.append(&mut view.messages);
    view.messages = messages;
}

/// The IN LINEAR field: who the issue is assigned to and its state, as the
/// sheet shows it (the survey's, or a re-read's after den asked again).
fn issue_field(writes: &Writes) -> SheetField {
    field(
        "IN LINEAR",
        linear_writes::describe(writes.shown.assignee.as_ref(), &writes.shown.state),
        None,
        Tone::Plain,
    )
}

/// The MOVE field: the toggle, or why there is no move.
fn move_field(sheet: &Sheet, linear: &LinearLaunch) -> SheetField {
    let writes = &linear.writes;
    match &writes.move_to {
        Some(to) if writes.move_on => field(
            "MOVE",
            format!("on: to {to}"),
            Some("m leaves the state as it is".to_owned()),
            Tone::Plain,
        ),
        Some(to) => field(
            "MOVE",
            "off".to_owned(),
            Some(format!("m moves it to {to}")),
            Tone::Muted,
        ),
        None => {
            let why = if sheet.next_step == Some(crate::domain::NextStep::Implement)
                && !linear_writes::not_started(&writes.shown.state)
            {
                "the issue has already started, so den leaves its state"
            } else {
                "the Linear Workflow names no in_progress_state"
            };
            field("MOVE", "none".to_owned(), Some(why.to_owned()), Tone::Muted)
        }
    }
}

/// What the Linear writes did, in order.
fn result_messages(writes: &Writes) -> Vec<SheetMessage> {
    writes
        .results
        .iter()
        .map(|result| match result {
            WriteResult::Done(text) => message(text, Tone::Good),
            WriteResult::Failed(text) | WriteResult::Unknown(text) => message(text, Tone::Caution),
            WriteResult::Read(text) => message(text, Tone::Plain),
        })
        .collect()
}

/// The Mark for release sheet (#38), drawn as a launch sheet without a prompt
/// or a command.
pub fn build_release(sheet: &ReleaseSheet) -> LaunchSheet {
    let mut view = LaunchSheet {
        label: RELEASE_LABEL.to_owned(),
        reference: sheet.reference.clone(),
        title: sheet.title.clone(),
        next_step: crate::domain::NextStep::MarkForRelease.label().to_owned(),
        status: LaunchStatus::Ready,
        headline: String::new(),
        fields: Vec::new(),
        reviewers: None,
        prompt: None,
        linear_writes: Vec::new(),
        command: None,
        messages: Vec::new(),
    };
    (view.status, view.headline) = match &sheet.state {
        ReleaseState::Ready => (
            LaunchStatus::Ready,
            "Ready: ⏎ moves it in Linear, esc cancels".to_owned(),
        ),
        ReleaseState::Writing => (LaunchStatus::Writing, "Writing to Linear…".to_owned()),
        ReleaseState::Moved => (LaunchStatus::Launched, "Moved in Linear".to_owned()),
        ReleaseState::Stopped => (LaunchStatus::Failed, "Stopped: see below".to_owned()),
        ReleaseState::Refused(_) => (
            LaunchStatus::Refused,
            "Refused: nothing was done".to_owned(),
        ),
    };
    if let Some(writes) = &sheet.writes {
        view.fields.push(issue_field(writes));
        if let Some(to) = &writes.move_to {
            view.fields.push(field(
                "RELEASE",
                to.clone(),
                Some("the Linear Workflow's release_state".to_owned()),
                Tone::Plain,
            ));
        }
        view.linear_writes = writes.planned();
    }
    view.fields
        .push(field("URL", sheet.url.clone(), None, Tone::Plain));
    if let ReleaseState::Refused(why) = &sheet.state {
        view.messages.push(message(why, Tone::Caution));
    }
    if let Some(writes) = &sheet.writes {
        view.messages.extend(result_messages(writes));
        if sheet.state == ReleaseState::Ready {
            view.messages.push(message(NO_COMPARE_AND_SET, Tone::Muted));
        }
    }
    if let Some(notice) = &sheet.notice {
        view.messages.push(message(notice, Tone::Caution));
    }
    // Outcomes and warnings must remain visible when release evidence is long.
    view.messages
        .extend(sheet.evidence.iter().map(|e| message(e, Tone::Plain)));
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
        Some(channel) if !request.announces => field(
            "ANNOUNCE",
            "nowhere".to_owned(),
            Some(format!(
                "the request_review template in prompts.toml has no {{announcement}}: the prompt neither announces in {channel} nor asks for the request's outcome"
            )),
            Tone::Caution,
        ),
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

/// A Land sheet's members (#40): `LAND 1`, `LAND 2`, … in landing order,
/// each with its Repo, PR, head commit and checks; then each held member as
/// "not landed: held".
fn landing_fields(group: &LandingGroupPlan) -> Vec<SheetField> {
    let landed = group.members.iter().enumerate().map(|(i, member)| {
        let order = match member.configured {
            Some(rank) => format!("; landing_order position {}", rank + 1),
            None => String::new(),
        };
        field(
            &format!("LAND {}", i + 1),
            format!(
                "{} @ {}",
                landing::reference(&member.pr),
                short_commit(&member.head_commit)
            ),
            Some(format!("{}{order}", member.checks)),
            Tone::Plain,
        )
    });
    let held = group.held.iter().map(|member| {
        field(
            "HELD",
            landing::reference(&member.pr),
            Some(format!(
                "not landed: held (Held Repo {}); the Job is told never to merge it",
                member.pr.repo
            )),
            Tone::Muted,
        )
    });
    landed.chain(held).collect()
}

/// A Merge or Land sheet's resolution comments (#78): `COMMENT` (or
/// `COMMENT 1`, `COMMENT 2`, … in landing order) with its Points' tally. The
/// text itself is in the prompt, which `prompt` is as shown: an edit that no
/// longer carries a comment word for word is a caution, since the Job posts
/// what the prompt says.
fn resolution_fields(resolutions: &[Resolution], prompt: &str) -> Vec<SheetField> {
    let numbered = resolutions.len() > 1;
    resolutions
        .iter()
        .enumerate()
        .map(|(i, resolution)| {
            let label = if numbered {
                format!("COMMENT {}", i + 1)
            } else {
                "COMMENT".to_owned()
            };
            if prompt.contains(&resolution.body) {
                field(
                    &label,
                    resolution.tally.clone(),
                    Some(format!(
                        "posted verbatim on {} immediately before it merges; the text is in the prompt",
                        resolution.url
                    )),
                    Tone::Plain,
                )
            } else {
                field(
                    &label,
                    resolution.tally.clone(),
                    Some(format!(
                        "the edited prompt no longer carries den's comment for {} word for word",
                        resolution.url
                    )),
                    Tone::Caution,
                )
            }
        })
        .collect()
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

/// The PROJECT SETTINGS field when den turns the worktree's settings off
/// (`handoff::trust`); none when they load, as in milestone 1.
fn project_settings_field(settings: &ProjectSettings) -> Option<SheetField> {
    let text = settings.sheet_text()?;
    Some(field(
        "PROJECT SETTINGS",
        text,
        Some(format!(
            "{}: the worktree's .claude/settings.json and settings.local.json are not loaded; your own settings are",
            settings.sources().args().join(" ")
        )),
        Tone::Caution,
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
    if let Some(group) = &plan.landing {
        fields.extend(landing_fields(group));
    }
    fields.extend(resolution_fields(&plan.resolutions, &sheet.prompt.text()));
    fields.extend(permissions_field(plan.permission_mode));
    fields.extend(project_settings_field(&plan.project_settings));
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
    let land_job = job.subject.landing_group().map(|group| {
        field(
            "LAND JOB",
            format!("the Landing Group of {}", group.identifier),
            Some(format!(
                "it lands {}, pinned to their approved heads",
                group.members_text()
            )),
            Tone::Plain,
        )
    });
    land_job
        .into_iter()
        .chain([
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
        ])
        .collect()
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
    view.fields
        .extend(project_settings_field(&plan.project_settings));
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
    if let Some(first) = plan.landing.as_ref().and_then(|g| g.members.first()) {
        let head = match &plan.base.reference {
            BaseRef::PullHead { number } => format!("refs/pull/{number}/head"),
            BaseRef::HeadBranch(head) | BaseRef::DefaultBranch(head) => head.clone(),
        };
        return format!(
            "PR head {head} of {}, the first to land",
            landing::reference(&first.pr)
        );
    }
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
