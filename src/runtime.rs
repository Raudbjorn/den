//! Running the App: performing its Effects through the ports, either on a
//! background worker (the interactive TUI, which must stay responsive while `gh`
//! runs) or synchronously until nothing is pending (`den --dump`, and the seam-2
//! test harness).

use crate::{
    app::{App, Effect, Event, LogLevel, LogRecord, ViewModel},
    ports::{Ports, perform},
};
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread,
};

/// How many Effects may wait for the worker before new ones are refused.
const EFFECT_QUEUE_CAPACITY: usize = 16;
/// How many finished-operation Events may wait for the UI thread.
const EVENT_QUEUE_CAPACITY: usize = 16;

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

/// What [`dispatch`] leaves for the UI thread.
#[derive(Debug, Default)]
pub struct Dispatched {
    /// `Effect::Quit` was among the Effects.
    pub quit: bool,
    /// The Events of Effects performed at once, for the App next.
    pub events: Vec<Event>,
}

/// Effects performed on the UI thread instead of queued: small local writes the
/// user expects done once the screen says so. On the worker they would wait
/// behind a survey, and quitting does not wait for the worker.
fn performs_at_once(effect: &Effect) -> bool {
    matches!(effect, Effect::StoreUpstreams { .. })
}

/// Sends the App's Effects where they run: Quit and Log on the UI thread, local
/// setting writes at once ([`performs_at_once`]), everything else to `worker`.
pub fn dispatch(effects: Vec<Effect>, worker: &Worker, ports: &Ports) -> Dispatched {
    let mut out = Dispatched::default();
    for effect in effects {
        match effect {
            Effect::Quit => out.quit = true,
            Effect::Log(record) => log(&record),
            other if performs_at_once(&other) => out.events.extend(perform(&other, ports)),
            other => {
                if let Err(error) = worker.submit(other) {
                    tracing::error!(%error, "could not queue an effect");
                }
            }
        }
    }
    out
}

/// A background thread that performs Effects and sends back their Events.
pub struct Worker {
    effects: SyncSender<Effect>,
    events: Receiver<Event>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("den is still busy with earlier requests; try again shortly")]
    Busy,
    #[error("den's background worker stopped")]
    Stopped,
}

impl Worker {
    pub fn start(ports: Ports) -> Self {
        let (effect_tx, effect_rx) = mpsc::sync_channel::<Effect>(EFFECT_QUEUE_CAPACITY);
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
            effects: effect_tx,
            events: event_rx,
        }
    }

    /// Queues an Effect for the worker without blocking the UI.
    pub fn submit(&self, effect: Effect) -> Result<(), WorkerError> {
        self.effects.try_send(effect).map_err(|e| match e {
            TrySendError::Full(_) => WorkerError::Busy,
            TrySendError::Disconnected(_) => WorkerError::Stopped,
        })
    }

    /// Events the worker has finished, without waiting.
    pub fn finished(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }
}
