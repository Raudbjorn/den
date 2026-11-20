//! Shared helpers for den's integration tests.
//!
//! - `branch_graphql`: a scripted GitHub for the Branch fetcher's queries.
//! - `build`: builders for Observations and facts (one file per Work Item kind).
//! - `fakes`: fake ports (GitHub source, Workspace, Launcher, Clock) and a
//!   scripted GraphQL transport for the GitHub adapter.
//! - `harness`: drives the App through events with fake ports (test seam 2).
//! - `linear_replies`: scripted Linear replies and a source wired to them.
//! - `render` / `row_text` / `profile`: rendering through the engine.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code, unused_imports)]

pub mod branch_graphql;
pub mod build;
pub mod fakes;
pub mod harness;
pub mod linear_replies;

use crepuscularity_tui::TemplateContext;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect};
use raudbjorn_tui::{
    component::{ComponentRenderer, TemplateRef, TemplateSource, TemplateStore},
    profile::{ColorProfile, GlyphProfile, TerminalProfile},
};
use std::sync::Arc;

pub fn profile(color: ColorProfile) -> TerminalProfile {
    TerminalProfile::new(color, GlyphProfile::Unicode)
}

/// Renders `section` from the given template files through the engine, with
/// den's Oxide palette for the profile (the same pipeline `den::ui::Theme` uses).
pub fn render(
    sources: Vec<TemplateSource>,
    section: TemplateRef,
    ctx: &TemplateContext,
    profile: TerminalProfile,
    width: u16,
    height: u16,
) -> Buffer {
    let store = TemplateStore::from_sources(sources).expect("templates load");
    let renderer = ComponentRenderer::new(Arc::new(store));
    let palette = den::palette::oxide_for(profile.color);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| {
            renderer
                .render(
                    section,
                    ctx,
                    palette,
                    profile,
                    frame,
                    Rect::new(0, 0, width, height),
                )
                .expect("render");
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

pub fn row_text(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
}
