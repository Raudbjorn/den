//! The header plate: brand, Scope (sentence and segmented control), survey
//! status, coverage.

use super::{
    Theme, UiError,
    glyphs::{self, Glyphs},
    templates,
};
use crate::app::view_model::{Header, ScopeSegment};
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{Frame, layout::Rect};

const BRAND: &str = "DEN";
/// Columns taken on the Scope row besides the sentence and the segments:
/// padding (2 each side), the brand (5) and the divider (3).
const SCOPE_ROW_CHROME_COLS: usize = 2 + 2 + 5 + 3;
/// Width of one Scope segment, in columns (template: `w-[16]`).
const SEGMENT_COLS: usize = 16;
/// The gap before each segment, in columns (template: `w-[1]`).
const SEGMENT_GAP_COLS: usize = 1;
/// Room for the active marker and a space before a segment's label.
const SEGMENT_MARK_COLS: usize = 2;

pub fn draw(frame: &mut Frame, theme: &Theme, header: &Header, area: Rect) -> Result<(), UiError> {
    let glyphs = glyphs::for_profile(theme.profile().glyphs);
    let mut ctx = TemplateContext::default();
    ctx.set("brand", BRAND)
        .set("scope", glyphs.text(&header.scope).as_ref())
        .set("scopes", segments(header, area.width, &glyphs))
        .set("status", glyphs.text(&header.status).as_ref())
        .set("coverage", glyphs.text(&header.coverage).as_ref())
        .set("coverage_complete", header.coverage_complete)
        .set("complete_glyph", glyphs.complete)
        .set("incomplete_glyph", glyphs.incomplete);
    theme.draw(frame, templates::HEADER, &ctx, area)
}

/// The segmented control, or nothing when it would crowd out the Scope
/// sentence (which always says the Scope in words).
fn segments(header: &Header, width: u16, glyphs: &Glyphs) -> TemplateValue {
    let needed = SCOPE_ROW_CHROME_COLS
        + header.scope.chars().count()
        + header.scopes.len() * (SEGMENT_GAP_COLS + SEGMENT_COLS);
    if needed > usize::from(width) {
        return TemplateValue::List(Vec::new());
    }
    TemplateValue::List(
        header
            .scopes
            .iter()
            .map(|segment| segment_context(segment, glyphs))
            .collect(),
    )
}

/// The active segment carries the tick glyph as well as its fill.
fn segment_context(segment: &ScopeSegment, glyphs: &Glyphs) -> TemplateContext {
    let room = SEGMENT_COLS - SEGMENT_MARK_COLS;
    let label: String = if segment.label.chars().count() > room {
        let kept = room.saturating_sub(glyphs.ellipsis.chars().count());
        let mut cut: String = segment.label.chars().take(kept).collect();
        cut.push_str(glyphs.ellipsis);
        cut
    } else {
        segment.label.clone()
    };
    let mark = if segment.active { glyphs.tick } else { " " };
    let mut ctx = TemplateContext::default();
    ctx.set("active", segment.active)
        .set("segment", format!("{mark}{label:^room$} "));
    ctx
}
