//! The Mark for release sheet (#38): `c` on a Linear Issue whose Next Step is
//! Mark for release. It names the issue, its state now and the Linear
//! Workflow's release state, lists the merged and closed members as evidence
//! (the Decision's own Explanation) and says den can't see deployments.
//! `Enter` re-reads the issue and, if its state is still what the sheet
//! showed, moves it to the release state ([`super::linear_writes`]), then
//! shows what Linear returned. A changed state asks again. Nothing else is
//! written, and no Job is launched: the move is den's own Linear write.
//!
//! Keys: `Enter` moves (then closes once done), `Esc` cancels (not while den
//! is writing), `Ctrl-C` quits.

use super::{
    App, Effect, Key, LogLevel, OverlayState,
    event::LaunchId,
    linear_writes::{Next, Shown, Watch, Writes},
};
use crate::{
    decide::Decision,
    domain::{NextStep, WorkItemId},
};

/// One open Mark for release sheet.
#[derive(Debug, Clone)]
pub(super) struct ReleaseSheet {
    pub id: LaunchId,
    pub item: WorkItemId,
    /// The Linear identifier.
    pub reference: String,
    pub title: String,
    pub url: String,
    /// The move to the release state; `None` when the sheet refused.
    pub writes: Option<Writes>,
    /// What the survey last said of the issue, as the sheet synced it.
    observed: Option<Shown>,
    /// The Decision's evidence and caveats: merged and closed members, and
    /// that den can't see deployments.
    pub evidence: Vec<String>,
    pub state: ReleaseState,
    /// A one-off message, e.g. why `Enter` asked again.
    pub notice: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReleaseState {
    Ready,
    Writing,
    /// The move was done; what Linear returned is in the writes' results.
    Moved,
    /// The run stopped: a failure or an unknown outcome.
    Stopped,
    Refused(String),
}

impl App {
    /// `c` on Mark for release: opens the sheet. Writes nothing.
    pub(super) fn open_release_sheet(&mut self, decision: &Decision) -> Vec<Effect> {
        let id = LaunchId(self.next_launch);
        self.next_launch += 1;
        let release = self
            .linear_writes_for(decision, Watch::State)
            .ok_or_else(|| "den no longer has this Linear Issue from Linear".to_owned())
            .and_then(|(writes, workflow)| match workflow.release_state {
                Some(release) => Ok((writes, release)),
                None => Err(format!(
                    "{}'s Linear Workflow names no release_state, so den has no state to move it to",
                    workflow.owner
                )),
            });
        let (writes, state) = match release {
            Ok((mut writes, release)) => {
                writes.move_to = Some(release);
                (Some(writes), ReleaseState::Ready)
            }
            Err(why) => (None, ReleaseState::Refused(why)),
        };
        let mut evidence: Vec<String> = decision.explanation.winning_rule.iter().cloned().collect();
        evidence.extend(decision.explanation.other_conditions.iter().cloned());
        evidence.extend(
            decision
                .explanation
                .unknowns
                .iter()
                .map(|u| format!("unknown: {u}")),
        );
        let log = Effect::log(
            LogLevel::Info,
            format!(
                "Mark for release sheet {} opened for {}",
                id.0,
                decision.item.reference()
            ),
        );
        self.overlay = Some(OverlayState::Release(Box::new(ReleaseSheet {
            id,
            item: decision.item.id.clone(),
            reference: decision.item.reference(),
            title: decision.item.title.clone(),
            url: decision.item.url.clone(),
            observed: writes.as_ref().map(|w| w.shown.clone()),
            writes,
            evidence,
            state,
            notice: None,
        })));
        vec![log]
    }

    /// Keys while the Mark for release sheet is open; it claims all of them.
    pub(super) fn release_key(&mut self, mut sheet: Box<ReleaseSheet>, key: Key) -> Vec<Effect> {
        if key == Key::Interrupt {
            return vec![Effect::Quit];
        }
        sheet.notice = None;
        let mut effects = Vec::new();
        match (key, &sheet.state) {
            (Key::Esc | Key::Enter, ReleaseState::Writing) => {
                sheet.notice = Some("den is writing to Linear; wait for it to finish".to_owned());
            }
            (Key::Enter, ReleaseState::Ready) => effects = self.release_confirmed(&mut sheet),
            // Closed: nothing was or will be done by this sheet any more.
            (Key::Esc, _)
            | (
                Key::Enter,
                ReleaseState::Moved | ReleaseState::Stopped | ReleaseState::Refused(_),
            ) => {
                return Vec::new();
            }
            _ => {}
        }
        self.overlay = Some(OverlayState::Release(sheet));
        effects
    }

    /// `Enter` on a ready sheet: asks again when a survey changed the issue or
    /// its Next Step since the sheet showed it; otherwise re-reads, then moves.
    fn release_confirmed(&self, sheet: &mut ReleaseSheet) -> Vec<Effect> {
        let step = self.decision_of(&sheet.item).and_then(|d| d.next_step);
        if step != Some(NextStep::MarkForRelease) {
            let now = step.map_or("none", NextStep::label);
            sheet.state = ReleaseState::Refused(format!(
                "the Next Step is now {now}, not Mark for release: nothing was written"
            ));
            return vec![Effect::log(
                LogLevel::Info,
                format!(
                    "Mark for release sheet {} for {}: the Next Step is now {now}",
                    sheet.id.0, sheet.reference
                ),
            )];
        }
        let surveyed = self.linear_issue(&sheet.item).map(|issue| Shown {
            assignee: issue.assignee.clone(),
            state: issue.state.clone(),
        });
        let Some(writes) = sheet.writes.as_mut() else {
            return Vec::new();
        };
        if let Some(now) = surveyed
            && sheet
                .observed
                .as_ref()
                .is_some_and(|o| o.state != now.state)
        {
            sheet.notice = Some(format!(
                "Nothing written: the survey now shows {} in {} (was {}). Check the sheet, then press Enter to move it",
                sheet.reference, now.state.name, writes.shown.state.name
            ));
            writes.shown = now.clone();
            sheet.observed = Some(now);
            return Vec::new();
        }
        match writes.begin(sheet.id) {
            Next::Wait(effects) => {
                sheet.state = ReleaseState::Writing;
                effects
            }
            // Nothing to write can't happen here (the move is always on).
            _ => Vec::new(),
        }
    }

    /// Whether the open sheet is Mark for release sheet `id`.
    pub(super) fn release_sheet_is(&self, id: LaunchId) -> bool {
        matches!(&self.overlay, Some(OverlayState::Release(sheet)) if sheet.id == id)
    }

    /// Advances the Mark for release sheet `id`'s writes by `step`.
    pub(super) fn release_step(
        &mut self,
        id: LaunchId,
        step: impl FnOnce(&mut Writes) -> Next,
    ) -> Vec<Effect> {
        let Some(OverlayState::Release(mut sheet)) = self.overlay.take() else {
            return Vec::new();
        };
        let Some(writes) = sheet.writes.as_mut() else {
            self.overlay = Some(OverlayState::Release(sheet));
            return Vec::new();
        };
        let reference = sheet.reference.clone();
        let effects = match step(writes) {
            Next::Wait(effects) => effects,
            Next::Done => {
                sheet.state = ReleaseState::Moved;
                vec![Effect::log(
                    LogLevel::Info,
                    format!("Mark for release sheet {} moved {reference}", id.0),
                )]
            }
            Next::AskAgain(change) => {
                sheet.notice = Some(format!(
                    "Nothing written: {change} since the sheet showed it. Check the sheet, then press Enter to move it"
                ));
                sheet.state = ReleaseState::Ready;
                vec![Effect::log(
                    LogLevel::Info,
                    format!(
                        "Mark for release sheet {} for {reference}: {change}; asking again",
                        id.0
                    ),
                )]
            }
            Next::Stopped => {
                sheet.state = ReleaseState::Stopped;
                vec![Effect::log(
                    LogLevel::Warn,
                    format!("Mark for release sheet {} for {reference}: stopped", id.0),
                )]
            }
        };
        self.overlay = Some(OverlayState::Release(sheet));
        effects
    }
}
