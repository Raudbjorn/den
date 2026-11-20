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

    /// Queues an Effect without blocking; feed any rejection event back to the App.
    pub fn submit(&self, effect: Effect) -> Result<(), Box<Event>> {
        self.effects.try_send(effect).map_err(|e| {
            let (effect, error) = match e {
                TrySendError::Full(effect) => (effect, WorkerError::Busy),
                TrySendError::Disconnected(effect) => (effect, WorkerError::Stopped),
            };
            Box::new(Event::EffectRejected {
                effect: Box::new(effect),
                reason: error.to_string(),
            })
        })
    }

    /// Events the worker has finished, without waiting.
    pub fn finished(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_or_stopped_worker_returns_the_rejected_effect_to_the_app() {
        let (effects, queued) = mpsc::sync_channel(EFFECT_QUEUE_CAPACITY);
        let (_events, events) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let worker = Worker { effects, events };
        for _ in 0..EFFECT_QUEUE_CAPACITY {
            assert!(worker.submit(Effect::LoadCache).is_ok());
        }
        let effect = Effect::StoreUpstreams {
            upstreams: Default::default(),
        };
        let rejected = worker.submit(effect.clone()).unwrap_err();
        assert!(
            matches!(*rejected, Event::EffectRejected { effect: returned, reason }
            if *returned == effect && reason.contains("still busy"))
        );
        assert_eq!(queued.try_iter().count(), EFFECT_QUEUE_CAPACITY);
        assert!(worker.submit(Effect::CheckJobs).is_ok());
        drop(queued);
        let rejected = worker.submit(effect.clone()).unwrap_err();
        assert!(
            matches!(*rejected, Event::EffectRejected { effect: returned, reason }
            if *returned == effect && reason.contains("worker stopped"))
        );
    }
}
