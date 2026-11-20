//! The footer: key hints.

use super::{Theme, UiError, glyphs, templates};
use crate::app::view_model::Footer;
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{Frame, layout::Rect};
use unicode_width::UnicodeWidthStr;

/// Columns between a hint's key and its label.
const KEY_GAP_COLS: usize = 1;
/// Columns after a hint's label, before the next hint. Each hint takes only
/// its own width, so all seven sidebar hints fit in 80 columns.
const FOOTER_GAP_COLS: usize = 2;

pub fn draw(frame: &mut Frame, theme: &Theme, footer: &Footer, area: Rect) -> Result<(), UiError> {
    let glyphs = glyphs::for_profile(theme.profile().glyphs);
    let hints = footer
        .hints
        .iter()
        .map(|hint| {
            let key = glyphs.text(&hint.key);
            let mut ctx = TemplateContext::default();
            ctx.set("key", key.as_ref())
                .set("key_cols", (key.width() + KEY_GAP_COLS).to_string())
                .set("label", hint.label.as_str())
                .set(
                    "label_cols",
                    (hint.label.width() + FOOTER_GAP_COLS).to_string(),
                );
            ctx
        })
        .collect();
    let mut ctx = TemplateContext::default();
    ctx.set("hints", TemplateValue::List(hints));
    theme.draw(frame, templates::FOOTER, &ctx, area)
}
