//! Carrying out one Effect through the ports. Shared by `main`'s background
//! worker and the seam-2 test harness, so tests exercise the same wiring.

use super::Ports;
use crate::{
    app::{Effect, Event},
    handoff,
};

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
        Effect::LoadCache => vec![Event::CacheLoaded {
            outcome: ports.cache.load().map(|c| c.map(Box::new)),
        }],
        Effect::StoreCache { cached } => vec![Event::CacheStored {
            outcome: ports.cache.store(cached),
        }],
        Effect::InspectLaunch { id, request } => vec![Event::LaunchInspected {
            id: *id,
            view: Box::new(handoff::inspect(
                request,
                ports.workspace.as_ref(),
                ports.launcher.as_ref(),
                ports.jobs.as_ref(),
            )),
        }],
        Effect::Launch { id, plan } => vec![Event::LaunchFinished {
            id: *id,
            report: Box::new(handoff::execute(
                plan,
                ports.workspace.as_ref(),
                ports.launcher.as_ref(),
                ports.jobs.as_ref(),
                ports.clock.now(),
            )),
        }],
        Effect::Quit | Effect::Log(_) => Vec::new(),
    }
}
