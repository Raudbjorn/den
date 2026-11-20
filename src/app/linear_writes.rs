//! The Linear writes a sheet makes (#38, ADR-0004): assigning a Linear Issue
//! to the Linear viewer and moving its state, each only after `Enter` on a
//! sheet that showed it.
//!
//! One run, in order, stopping at the first failure:
//!
//! 1. **Re-read** the issue (`Effect::ReadLinearIssue`). If its assignee or
//!    state (what the sheet [`Watch`]es) differs from what the sheet showed,
//!    den writes nothing: the sheet shows the issue as it is now and asks
//!    again. The re-read also names the state to move to by ID, from the
//!    issue's own team, whichever team that is.
//! 2. **Assign** to the Linear viewer, when the sheet claims the issue.
//! 3. **Move** to the chosen state, when the sheet moves it.
//!
//! Then the sheet carries on (a Start or Implement sheet launches the Job; the
//! Mark for release sheet is done). A failed step stops the run and nothing
//! after it happens; den never un-assigns or moves an issue back. A write whose
//! answer was lost (**unknown**) is followed by one more re-read, before
//! anything else, to show what Linear says now, and the run stops there: den
//! never retries a write blind. There is no compare-and-set in Linear, so a
//! claim can still land in between; den shows what Linear returned.

use super::{Effect, LogLevel, event::LaunchId};
use crate::{
    domain::{
        LinearViewer,
        facts::{LinearState, LinearStateKind, LinearUser},
    },
    ports::{
        LinearIssueNow, LinearIssueRead, LinearIssueRequest, LinearWriteKind, LinearWriteOutcome,
    },
};

/// What the sheet showed of the issue: what a re-read is compared with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Shown {
    pub assignee: Option<LinearUser>,
    pub state: LinearState,
}

/// Which changes since the sheet showed the issue make den ask again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Watch {
    /// Start and Implement: a new assignee or a new state.
    AssigneeAndState,
    /// Mark for release: a new state.
    State,
}

/// How far a run got.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Rereading,
    Assigning,
    Moving,
    /// A write's outcome was unknown: one more re-read shows what Linear says
    /// now, and the run stops.
    RereadingAfterUnknown,
}

/// What one step of the run ended as, shown on the sheet in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WriteResult {
    Done(String),
    Failed(String),
    Unknown(String),
    /// What Linear showed when den read the issue again.
    Read(String),
}

/// What the sheet does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Next {
    /// Perform these Effects and wait for the answer.
    Wait(Vec<Effect>),
    /// Every write is done (or there were none): carry on.
    Done,
    /// The issue changed since the sheet showed it; nothing was written. The
    /// sheet now shows it as it is, and `Enter` asks again.
    AskAgain(String),
    /// The run stopped; nothing more is done.
    Stopped,
}

/// A sheet's Linear writes and their run.
#[derive(Debug, Clone)]
pub(super) struct Writes {
    pub request: LinearIssueRequest,
    /// The Linear viewer of the issue's workspace, when den read it.
    pub viewer: Option<LinearViewer>,
    pub shown: Shown,
    pub watch: Watch,
    /// Assign the issue to the Linear viewer (Start's claim).
    pub claim: bool,
    /// The state the move goes to, by name, when the sheet offers a move.
    pub move_to: Option<String>,
    /// The move's toggle: on by default.
    pub move_on: bool,
    phase: Option<Phase>,
    /// The state the run moves to, by the re-read's ID.
    resolved: Option<LinearState>,
    /// This run's assignment and move were done.
    assigned: bool,
    moved: bool,
    /// Every write was done: a second `Enter` (after the base moved) launches
    /// without writing again.
    pub finished: bool,
    pub results: Vec<WriteResult>,
}

impl Writes {
    pub fn new(
        request: LinearIssueRequest,
        viewer: Option<LinearViewer>,
        shown: Shown,
        watch: Watch,
    ) -> Self {
        Self {
            request,
            viewer,
            shown,
            watch,
            claim: false,
            move_to: None,
            move_on: true,
            phase: None,
            resolved: None,
            assigned: false,
            moved: false,
            finished: false,
            results: Vec::new(),
        }
    }

    /// The move will be made: offered and on.
    pub fn moves(&self) -> bool {
        self.move_on && self.move_to.is_some()
    }

    /// `Enter` writes something before carrying on.
    pub fn pending(&self) -> bool {
        !self.finished && (self.claim || self.moves())
    }

    /// The identifier, for messages.
    fn identifier(&self) -> &str {
        &self.request.identifier
    }

    /// Who the claim assigns the issue to, in words.
    pub fn me(&self) -> String {
        match &self.viewer {
            Some(viewer) => format!("{} (you, the Linear viewer)", viewer.name),
            None => "you (the Linear viewer)".to_owned(),
        }
    }

    /// The writes `Enter` will make, one line each, in order.
    pub fn planned(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.finished {
            return lines;
        }
        if self.claim {
            lines.push(format!(
                "Linear: assign {} to {}",
                self.identifier(),
                self.me()
            ));
        }
        if let (true, Some(to)) = (self.move_on, &self.move_to) {
            lines.push(format!(
                "Linear: move {} from {} to {to}",
                self.identifier(),
                self.shown.state.name
            ));
        }
        lines
    }

    /// The run begins (`Enter`): re-read first. `Done` when nothing is to be
    /// written.
    pub fn begin(&mut self, id: LaunchId) -> Next {
        if !self.pending() {
            return Next::Done;
        }
        self.results.clear();
        self.resolved = None;
        self.assigned = false;
        self.moved = false;
        self.phase = Some(Phase::Rereading);
        Next::Wait(vec![
            self.log(
                id,
                LogLevel::Info,
                "confirmed; reading it again before writing",
            ),
            Effect::ReadLinearIssue {
                id,
                request: Box::new(self.request.clone()),
            },
        ])
    }

    /// The issue was read again.
    pub fn reread(&mut self, id: LaunchId, read: LinearIssueRead) -> Next {
        match self.phase.take() {
            Some(Phase::Rereading) => self.compare(id, read),
            Some(Phase::RereadingAfterUnknown) => {
                self.results.push(match read {
                    LinearIssueRead::Read(now) => WriteResult::Read(format!(
                        "Linear now shows {} {}",
                        self.identifier(),
                        describe(now.assignee.as_ref(), &now.state)
                    )),
                    LinearIssueRead::Failed { error } => WriteResult::Failed(format!(
                        "den couldn't read {} again either ({error}): check it in Linear",
                        self.identifier()
                    )),
                });
                self.stopped()
            }
            // Not waiting for a read: a late answer changes nothing.
            other => {
                self.phase = other;
                Next::Wait(Vec::new())
            }
        }
    }

    /// The re-read before writing: unchanged → write; changed → ask again.
    fn compare(&mut self, id: LaunchId, read: LinearIssueRead) -> Next {
        let now = match read {
            LinearIssueRead::Read(now) => now,
            LinearIssueRead::Failed { error } => {
                self.results.push(WriteResult::Failed(format!(
                    "den couldn't read {} again, so it wrote nothing: {error}",
                    self.identifier()
                )));
                return self.stopped();
            }
        };
        if let Some(change) = self.changed(&now) {
            self.shown = Shown {
                assignee: now.assignee.clone(),
                state: now.state.clone(),
            };
            return Next::AskAgain(change);
        }
        if let (true, Some(name)) = (self.move_on, &self.move_to) {
            match now
                .team_states
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
            {
                Some(state) => self.resolved = Some(state.clone()),
                None => {
                    let read = if now.states_complete {
                        "has no state"
                    } else {
                        "has no state den read (it read only the first states)"
                    };
                    self.results.push(WriteResult::Failed(format!(
                        "{}'s team {read} named {name:?}, so den wrote nothing",
                        self.identifier()
                    )));
                    return self.stopped();
                }
            }
        }
        self.next_write(id)
    }

    /// What changed since the sheet showed the issue, if anything it watches.
    fn changed(&self, now: &LinearIssueNow) -> Option<String> {
        let assignee = |user: Option<&LinearUser>| user.map(|u| u.id.clone());
        if self.watch == Watch::AssigneeAndState
            && assignee(now.assignee.as_ref()) != assignee(self.shown.assignee.as_ref())
        {
            return Some(match &now.assignee {
                Some(user) => format!("{} is now assigned to {}", self.identifier(), user.name),
                None => format!("{} is now unassigned", self.identifier()),
            });
        }
        (now.state.id != self.shown.state.id).then(|| {
            format!(
                "{} is now in {} (was {})",
                self.identifier(),
                now.state.name,
                self.shown.state.name
            )
        })
    }

    /// The next write after the re-read or an earlier write, or `Done`.
    fn next_write(&mut self, id: LaunchId) -> Next {
        if self.claim && !self.assigned {
            self.phase = Some(Phase::Assigning);
            return Next::Wait(vec![
                self.log(id, LogLevel::Info, "assigning it to the Linear viewer"),
                Effect::AssignLinearIssue {
                    id,
                    request: Box::new(self.request.clone()),
                },
            ]);
        }
        if let (Some(state), false) = (self.resolved.clone(), self.moved) {
            self.phase = Some(Phase::Moving);
            return Next::Wait(vec![
                self.log(id, LogLevel::Info, &format!("moving it to {}", state.name)),
                Effect::MoveLinearIssue {
                    id,
                    request: Box::new(self.request.clone()),
                    state: Box::new(state),
                },
            ]);
        }
        self.finished = true;
        Next::Done
    }

    /// A write ended.
    pub fn written(
        &mut self,
        id: LaunchId,
        write: LinearWriteKind,
        outcome: LinearWriteOutcome,
    ) -> Next {
        let expected = match write {
            LinearWriteKind::AssignToViewer => Phase::Assigning,
            LinearWriteKind::MoveState => Phase::Moving,
        };
        if self.phase.as_ref() != Some(&expected) {
            // Not waiting for this write: a late answer changes nothing.
            return Next::Wait(Vec::new());
        }
        self.phase = None;
        let what = match write {
            LinearWriteKind::AssignToViewer => format!("assigning {}", self.identifier()),
            LinearWriteKind::MoveState => format!(
                "moving {} to {}",
                self.identifier(),
                self.resolved
                    .as_ref()
                    .map_or("its state", |s| s.name.as_str())
            ),
        };
        match outcome {
            LinearWriteOutcome::Done { assignee, state } => {
                let done = match write {
                    LinearWriteKind::AssignToViewer => {
                        self.assigned = true;
                        "assigned"
                    }
                    LinearWriteKind::MoveState => {
                        self.moved = true;
                        "moved"
                    }
                };
                let mut line = format!(
                    "{done} {}: Linear returned it {}",
                    self.identifier(),
                    describe(assignee.as_ref(), &state)
                );
                if write == LinearWriteKind::AssignToViewer
                    && let (Some(viewer), Some(user)) = (&self.viewer, &assignee)
                    && !viewer.is(&user.id)
                {
                    line.push_str(", not to you: someone else's claim landed in between");
                }
                self.results.push(WriteResult::Done(line));
                self.next_write(id)
            }
            LinearWriteOutcome::Failed { error } => {
                self.results
                    .push(WriteResult::Failed(format!("{what} failed: {error}")));
                self.stopped()
            }
            LinearWriteOutcome::Unknown { why } => {
                self.results.push(WriteResult::Unknown(format!(
                    "{what}: unknown ({why}); den reads it again and won't retry"
                )));
                self.phase = Some(Phase::RereadingAfterUnknown);
                Next::Wait(vec![
                    self.log(
                        id,
                        LogLevel::Warn,
                        &format!("{what}: outcome unknown ({why}); reading it again"),
                    ),
                    Effect::ReadLinearIssue {
                        id,
                        request: Box::new(self.request.clone()),
                    },
                ])
            }
        }
    }

    /// The Effect for the run couldn't be queued: it was never performed.
    pub fn not_queued(&mut self, id: LaunchId, effect: &Effect, reason: &str) -> Next {
        match effect {
            Effect::ReadLinearIssue { .. } => self.reread(
                id,
                LinearIssueRead::Failed {
                    error: format!("den couldn't ask Linear: {reason}"),
                },
            ),
            Effect::AssignLinearIssue { .. } => self.written(
                id,
                LinearWriteKind::AssignToViewer,
                LinearWriteOutcome::Failed {
                    error: format!("den couldn't send it: {reason}"),
                },
            ),
            Effect::MoveLinearIssue { .. } => self.written(
                id,
                LinearWriteKind::MoveState,
                LinearWriteOutcome::Failed {
                    error: format!("den couldn't send it: {reason}"),
                },
            ),
            _ => Next::Wait(Vec::new()),
        }
    }

    fn stopped(&mut self) -> Next {
        self.phase = None;
        Next::Stopped
    }

    fn log(&self, id: LaunchId, level: LogLevel, what: &str) -> Effect {
        Effect::log(
            level,
            format!(
                "sheet {} Linear writes for {}: {what}",
                id.0,
                self.identifier()
            ),
        )
    }
}

/// "assigned to Ada, in Todo" / "unassigned, in Todo".
pub(super) fn describe(assignee: Option<&LinearUser>, state: &LinearState) -> String {
    match assignee {
        Some(user) => format!("assigned to {}, in {}", user.name, state.name),
        None => format!("unassigned, in {}", state.name),
    }
}

/// Whether an issue in `state` hasn't started: Implement offers the move
/// only then (spec 0002, story 61).
pub(super) fn not_started(state: &LinearState) -> bool {
    matches!(
        state.kind,
        LinearStateKind::Triage | LinearStateKind::Backlog | LinearStateKind::Unstarted
    )
}
