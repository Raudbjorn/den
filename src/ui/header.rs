//! The header plate: brand, Scope, survey status, coverage.

use super::{Theme, UiError, glyphs, templates};
use crate::app::view_model::Header;
use crepuscularity_tui::TemplateContext;
use ratatui::{Frame, layout::Rect};

const BRAND: &str = "DEN";

pub fn draw(frame: &mut Frame, theme: &Theme, header: &Header, area: Rect) -> Result<(), UiError> {
    let glyphs = glyphs::for_profile(theme.profile().glyphs);
    let mut ctx = TemplateContext::default();
    ctx.set("brand", BRAND)
        .set("scope", header.scope.as_str())
        .set("status", header.status.as_str())
        .set("coverage", header.coverage.as_str())
        .set("coverage_complete", header.coverage_complete)
        .set("complete_glyph", glyphs.complete)
        .set("incomplete_glyph", glyphs.incomplete);
    theme.draw(frame, templates::HEADER, &ctx, area)
}
