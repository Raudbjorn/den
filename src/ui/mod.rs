//! Rendering: den's templates on the vendored engine, painted in IRONMAW Oxide.
//! It draws a [`ViewModel`](crate::app::ViewModel) and holds no logic about GitHub.
//! One module per plate (header, list, detail, footer); `frame` lays them out
//! for the ViewModel's layout; `bear` draws the bear.

pub mod bear;
mod detail;
mod footer;
pub mod frame;
mod glyphs;
mod header;
pub mod launch;
pub mod list;
pub mod sidebar;
pub mod templates;
mod text;

use crate::palette::oxide_for;
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{Frame, layout::Rect};
use raudbjorn_tui::{
    component::{ComponentError, ComponentRenderer, TemplateRef, TemplateStore},
    profile::TerminalProfile,
    theme::TerminalPalette,
};
use std::{cell::Cell, sync::Arc};

#[derive(Debug, thiserror::Error)]
pub enum UiError {
    #[error("den's templates failed to load: {0}")]
    Templates(#[source] ComponentError),
    #[error("rendering failed: {0}")]
    Render(#[source] ComponentError),
    #[error("could not render to a test buffer: {0}")]
    Buffer(#[source] std::io::Error),
}

/// What drawing asked of the template engine (#140), counted only inside
/// [`measure`], so drawing to the terminal pays nothing for it. Unlike wall
/// time it is the same on every machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrawWork {
    /// Template sections drawn: one per [`Theme::draw`].
    pub calls: usize,
    /// The cells those sections cover, summed: the engine visits every cell
    /// of each section's area after laying it out.
    pub cells: usize,
    /// The most context values (counted through lists and scopes) any one
    /// section was handed. The engine clones its context per element, so a
    /// section carrying every row costs the square of the rows on screen.
    pub largest_context: usize,
}

thread_local! {
    /// The work counted so far on this thread, while [`measure`] runs.
    static DRAW_WORK: Cell<Option<DrawWork>> = const { Cell::new(None) };
}

/// Runs `draw` and returns what it asked of the template engine on this
/// thread.
pub fn measure<T>(draw: impl FnOnce() -> T) -> (T, DrawWork) {
    let outer = DRAW_WORK.replace(Some(DrawWork::default()));
    let out = draw();
    let work = DRAW_WORK.replace(outer).unwrap_or_default();
    (out, work)
}

/// Counts one section drawn into `area` with `ctx`, while [`measure`] runs.
fn count_draw(ctx: &TemplateContext, area: Rect) {
    DRAW_WORK.with(|cell| {
        if let Some(mut work) = cell.get() {
            work.calls += 1;
            work.cells += usize::from(area.width) * usize::from(area.height);
            work.largest_context = work.largest_context.max(context_values(ctx));
            cell.set(Some(work));
        }
    });
}

/// The values in `ctx`, each list element's and scope's values included.
fn context_values(ctx: &TemplateContext) -> usize {
    ctx.vars
        .values()
        .map(|value| {
            1 + match value {
                TemplateValue::List(items) => items.iter().map(context_values).sum(),
                TemplateValue::Scope(scope) => context_values(scope),
                _ => 0,
            }
        })
        .sum()
}

/// The engine, den's templates and the Oxide palette for one terminal profile.
pub struct Theme {
    renderer: ComponentRenderer,
    profile: TerminalProfile,
    palette: TerminalPalette,
}

impl Theme {
    pub fn new(profile: TerminalProfile) -> Result<Self, UiError> {
        let store =
            TemplateStore::from_sources(templates::sources()).map_err(UiError::Templates)?;
        Ok(Self {
            renderer: ComponentRenderer::new(Arc::new(store)),
            profile,
            palette: oxide_for(profile.color),
        })
    }

    pub fn profile(&self) -> TerminalProfile {
        self.profile
    }

    /// Renders one template section into `area`, in Oxide, for this profile.
    pub fn draw(
        &self,
        frame: &mut Frame,
        section: TemplateRef,
        ctx: &TemplateContext,
        area: Rect,
    ) -> Result<(), UiError> {
        count_draw(ctx, area);
        self.renderer
            .render(section, ctx, self.palette, self.profile, frame, area)
            .map_err(UiError::Render)
    }
}
