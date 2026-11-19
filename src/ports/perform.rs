//! Carrying out one Effect through the ports. Shared by `main`'s background
//! worker and the seam-2 test harness, so tests exercise the same wiring.

use super::Ports;
use crate::app::{Effect, Event};

/// Performs `effect` and returns the events it produced (none for effects the
/// caller handles itself, such as Quit and Log).
pub fn perform(effect: &Effect, ports: &Ports) -> Vec<Event> {
    match effect {
        Effect::Survey { id, request } => {
            let outcome = ports.github.survey(request).map(Box::new);
            vec![Event::SurveyFinished {
                id: *id,
                outcome,
                now: ports.clock.now(),
            }]
        }
        Effect::Quit | Effect::Log(_) => Vec::new(),
    }
}
