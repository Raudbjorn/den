//! Running the App: performing its Effects through the ports, either on a
//! background [`Worker`] (the interactive TUI, which must stay responsive while
//! `gh` runs) or synchronously until nothing is pending (`den --dump`, and the
//! seam-2 test harness).
//!
//! The Worker has three lanes, each a thread with bounded queues ([`lane_of`]):
//! the store lane reads and writes den's own state files (cache, Upstream
//! choices), so a choice is saved at once instead of waiting behind a survey;
//! Linear surveys have their own lane so they cannot hold back GitHub;
//! the remote lane runs everything else (`gh`, git, kitty). The UI thread only
//! logs. On quit, [`Worker::finish`] waits a bounded time for the store lane to
//! write what is queued; remote work still running is abandoned.

use crate::{
    app::{App, Effect, Event, LogLevel, LogRecord, ViewModel},
    ports::{Ports, perform},
};
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    thread,
    time::{Duration, Instant},
};

/// How many Effects may wait for each lane before new ones are refused.
const EFFECT_QUEUE_CAPACITY: usize = 16;
/// How many Linear surveys may wait behind one that is still running.
const LINEAR_QUEUE_CAPACITY: usize = 2;
/// How many finished-operation Events may wait for the UI thread, per lane.
const EVENT_QUEUE_CAPACITY: usize = 16;
/// How long quitting waits for the store lane to write what is queued.
pub const FINISH_WITHIN_SECS: u64 = 3;

/// Writes a Log effect to the structured log.
pub fn log(record: &LogRecord) {
    match record.level {
        LogLevel::Info => tracing::info!(message = %record.message),
        LogLevel::Warn => tracing::warn!(message = %record.message),
        LogLevel::Error => tracing::error!(message = %record.message),
    }
}

/// Handles `event`, then performs every resulting Effect synchronously and feeds
/// the Events they produce back in, until nothing is pending. Returns the final
/// ViewModel and every Effect the App emitted, in order.
pub fn settle(app: &mut App, event: Event, ports: &Ports) -> (ViewModel, Vec<Effect>) {
    let (mut view, first) = app.handle(event);
    let mut pending: VecDeque<Effect> = first.into();
    let mut emitted = Vec::new();
    while let Some(effect) = pending.pop_front() {
        if let Effect::Log(record) = &effect {
            log(record);
        }
        for follow_up in perform(&effect, ports) {
            let (next_view, effects) = app.handle(follow_up);
            view = next_view;
            pending.extend(effects);
        }
        emitted.push(effect);
    }
    (view, emitted)
}

/// What the loop does after a batch of Effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

/// Which of the [`Worker`]'s lanes performs an Effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneKind {
    /// den's own state files: small local reads and writes, drained on quit.
    Store,
    /// `gh`, git and kitty: slow or external, abandoned on quit.
    Remote,
    /// Linear surveys: independent of GitHub and abandoned on quit.
    Linear,
}

/// The lane `effect` runs on, or `None` for the Effects the UI thread handles
/// itself. No wildcard: a new Effect must choose.
pub fn lane_of(effect: &Effect) -> Option<LaneKind> {
    match effect {
        Effect::LoadCache | Effect::StoreCache { .. } | Effect::StoreUpstreams { .. } => {
            Some(LaneKind::Store)
        }
        Effect::SurveyLinear { .. } => Some(LaneKind::Linear),
        Effect::Survey { .. }
        | Effect::ReadMemberStates { .. }
        | Effect::InspectLaunch { .. }
        | Effect::Launch { .. }
        | Effect::CheckJobs
        | Effect::Reopen { .. } => Some(LaneKind::Remote),
        Effect::Quit | Effect::Log(_) => None,
    }
}

/// One background thread performing Effects in order, with bounded queues both
/// ways. Its Event sender drops when the thread ends, so a disconnected
/// `events` means everything sent to it was performed.
struct Lane {
    /// `None` once closed: the thread finishes what is queued, then ends.
    effects: Option<SyncSender<Effect>>,
    events: Receiver<Event>,
}

impl Lane {
    fn start(ports: Ports, capacity: usize) -> Self {
        let (effect_tx, effect_rx) = mpsc::sync_channel::<Effect>(capacity);
        let (event_tx, event_rx) = mpsc::sync_channel::<Event>(EVENT_QUEUE_CAPACITY);
        thread::spawn(move || {
            for effect in effect_rx {
                for event in perform(&effect, &ports) {
                    if event_tx.send(event).is_err() {
                        return; // The UI is gone.
                    }
                }
            }
        });
        Self {
            effects: Some(effect_tx),
            events: event_rx,
        }
    }

    /// Queues an Effect without blocking; a full or stopped lane hands it back
    /// as its rejection Event.
    fn submit(&self, effect: Effect) -> Result<(), Box<Event>> {
        let rejected = |effect, error: WorkerError| {
            Box::new(Event::EffectRejected {
                effect: Box::new(effect),
                reason: error.to_string(),
            })
        };
        let Some(effects) = &self.effects else {
            return Err(rejected(effect, WorkerError::Stopped));
        };
        effects.try_send(effect).map_err(|e| match e {
            TrySendError::Full(effect) => rejected(effect, WorkerError::Busy),
            TrySendError::Disconnected(effect) => rejected(effect, WorkerError::Stopped),
        })
    }
}

/// Performs Effects in the background on three lanes (see the module doc) and
/// sends back their Events.
pub struct Worker {
    store: Lane,
    remote: Lane,
    linear: Lane,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("den is still busy with earlier requests; try again shortly")]
    Busy,
    #[error("den's background worker stopped")]
    Stopped,
}

/// Why [`Worker::finish`] returned before the store lane was done.
#[derive(Debug, thiserror::Error)]
pub enum FinishError {
    #[error(
        "den's state files were still being written after {waited:?}; \
         the last cache or Upstream change may not be saved"
    )]
    TimedOut { waited: Duration },
}

impl Worker {
    pub fn start(ports: Ports) -> Self {
        Self {
            store: Lane::start(ports.clone(), EFFECT_QUEUE_CAPACITY),
            remote: Lane::start(ports.clone(), EFFECT_QUEUE_CAPACITY),
            linear: Lane::start(ports, LINEAR_QUEUE_CAPACITY),
        }
    }

    /// Sends the App's Effects where they run: Quit and Log on the UI thread,
    /// the rest to their lane ([`lane_of`]). An Effect a lane rejects goes back
    /// to `app` as its rejection Event at once, before the caller's next Event,
    /// and the Effects that leads to are dispatched in turn.
    pub fn dispatch(&self, app: &mut App, view: &mut ViewModel, effects: Vec<Effect>) -> Flow {
        let mut flow = Flow::Continue;
        let mut pending: VecDeque<_> = effects.into();
        while let Some(effect) = pending.pop_front() {
            let lane = match lane_of(&effect) {
                None => {
                    match effect {
                        Effect::Quit => flow = Flow::Quit,
                        Effect::Log(record) => log(&record),
                        _ => {}
                    }
                    continue;
                }
                Some(LaneKind::Store) => &self.store,
                Some(LaneKind::Remote) => &self.remote,
                Some(LaneKind::Linear) => &self.linear,
            };
            if let Err(rejected) = lane.submit(effect) {
                let (next_view, effects) = app.handle(*rejected);
                *view = next_view;
                pending.extend(effects);
            }
        }
        flow
    }

    /// Events the lanes have finished, without waiting.
    pub fn finished(&self) -> impl Iterator<Item = Event> + '_ {
        self.store
            .events
            .try_iter()
            .chain(self.remote.events.try_iter())
            .chain(self.linear.events.try_iter())
    }

    /// Ends the Worker for quitting. Events already finished are handled first,
    /// so a survey that just answered is still cached. Then all lanes close,
    /// and this waits, at most `within`, for the store lane to perform what is
    /// queued, handing its Events to `app` so a failed write is logged. Effects
    /// emitted meanwhile are rejected (the lanes are closed) and logged by the
    /// App. Remote work is not waited for.
    pub fn finish(mut self, app: &mut App, within: Duration) -> Result<(), FinishError> {
        let deadline = Instant::now() + within;
        let done: Vec<Event> = self.finished().collect();
        for event in done {
            self.handle_while_finishing(app, event);
        }
        self.store.effects = None;
        self.remote.effects = None;
        self.linear.effects = None;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.store.events.recv_timeout(left) {
                Ok(event) => self.handle_while_finishing(app, event),
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(FinishError::TimedOut { waited: within });
                }
            }
        }
    }

    /// Hands `event` to `app` and dispatches what follows; nothing is drawn
    /// any more, and a Quit among them changes nothing.
    fn handle_while_finishing(&self, app: &mut App, event: Event) {
        let (mut view, effects) = app.handle(event);
        self.dispatch(app, &mut view, effects);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::Key, config::Config};
    use jiff::Timestamp;

    /// A lane whose queue the test holds instead of a thread.
    fn held_lane() -> (Lane, Receiver<Effect>, SyncSender<Event>) {
        let (effects, queued) = mpsc::sync_channel(EFFECT_QUEUE_CAPACITY);
        let (finished, events) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let lane = Lane {
            effects: Some(effects),
            events,
        };
        (lane, queued, finished)
    }

    #[test]
    fn a_full_or_stopped_lane_returns_the_rejected_effect_to_the_app() {
        let (mut lane, queued, _finished) = held_lane();
        for _ in 0..EFFECT_QUEUE_CAPACITY {
            assert!(lane.submit(Effect::LoadCache).is_ok());
        }
        let effect = Effect::StoreUpstreams {
            upstreams: Default::default(),
        };
        let rejected = lane.submit(effect.clone()).unwrap_err();
        assert!(
            matches!(*rejected, Event::EffectRejected { effect: returned, reason }
            if *returned == effect && reason.contains("still busy"))
        );
        assert_eq!(queued.try_iter().count(), EFFECT_QUEUE_CAPACITY);
        assert!(lane.submit(Effect::CheckJobs).is_ok());
        drop(queued);
        let rejected = lane.submit(effect.clone()).unwrap_err();
        assert!(
            matches!(*rejected, Event::EffectRejected { effect: returned, reason }
            if *returned == effect && reason.contains("worker stopped"))
        );
        lane.effects = None;
        let rejected = lane.submit(effect.clone()).unwrap_err();
        assert!(
            matches!(*rejected, Event::EffectRejected { effect: returned, reason }
            if *returned == effect && reason.contains("worker stopped"))
        );
    }

    /// A rejection reaches the App inside `dispatch`, before the caller hands
    /// it the next Event: a refresh pressed right after a rejected survey
    /// starts a new one instead of waiting for the survey that never ran.
    #[test]
    fn a_rejected_survey_is_undone_before_the_next_key() {
        let (store, _store_queue, _store_events) = held_lane();
        let (remote, remote_queue, _remote_events) = held_lane();
        let (linear, _linear_queue, _linear_events) = held_lane();
        drop(remote_queue); // The remote lane has stopped.
        let worker = Worker {
            store,
            remote,
            linear,
        };
        let mut app = App::new(Ok(Config {
            owners: vec!["octo".to_owned()],
            main_clone_root: None,
            main_clones: Default::default(),
            linear: Vec::new(),
        }));
        let (mut view, effects) = app.handle(Event::Started {
            now: Timestamp::UNIX_EPOCH,
        });
        assert!(effects.iter().any(|e| matches!(e, Effect::Survey { .. })));
        assert_eq!(
            worker.dispatch(&mut app, &mut view, effects),
            Flow::Continue
        );

        let (_, effects) = app.handle(Event::Key(Key::Char('r')));
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Survey { .. })),
            "the rejected survey still counted as running: {effects:?}"
        );
    }

    #[test]
    fn quit_and_log_stay_on_the_ui_thread_and_writes_go_to_the_store_lane() {
        assert_eq!(lane_of(&Effect::Quit), None);
        assert_eq!(lane_of(&Effect::log(LogLevel::Info, "x")), None);
        assert_eq!(lane_of(&Effect::LoadCache), Some(LaneKind::Store));
        assert_eq!(
            lane_of(&Effect::StoreUpstreams {
                upstreams: Default::default()
            }),
            Some(LaneKind::Store)
        );
        assert_eq!(lane_of(&Effect::CheckJobs), Some(LaneKind::Remote));
    }
}
