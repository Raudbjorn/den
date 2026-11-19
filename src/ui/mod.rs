//! Rendering: den's templates on the vendored engine, painted in IRONMAW Oxide.
//! It draws a [`ViewModel`](crate::app::ViewModel) and holds no logic about GitHub.
//! One module per plate (header, list, detail, footer); `frame` lays them out.

mod detail;
mod footer;
pub mod frame;
mod glyphs;
mod header;
pub mod list;
pub mod templates;
mod text;

use crate::palette::oxide_for;
use crepuscularity_tui::TemplateContext;
use ratatui::{Frame, layout::Rect};
use raudbjorn_tui::{
    component::{ComponentError, ComponentRenderer, TemplateRef, TemplateStore},
    profile::TerminalProfile,
    theme::TerminalPalette,
};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum UiError {
    #[error("den's templates failed to load: {0}")]
    Templates(#[source] ComponentError),
    #[error("rendering failed: {0}")]
    Render(#[source] ComponentError),
    #[error("could not render to a test buffer: {0}")]
    Buffer(#[source] std::io::Error),
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
        self.renderer
            .render(section, ctx, self.palette, self.profile, frame, area)
            .map_err(UiError::Render)
    }
}
