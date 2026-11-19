//! The footer: key hints.

use super::{Theme, UiError, templates};
use crate::app::view_model::Footer;
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{Frame, layout::Rect};

pub fn draw(frame: &mut Frame, theme: &Theme, footer: &Footer, area: Rect) -> Result<(), UiError> {
    let hints = footer
        .hints
        .iter()
        .map(|hint| {
            let mut ctx = TemplateContext::default();
            ctx.set("key", hint.key.as_str())
                .set("label", hint.label.as_str());
            ctx
        })
        .collect();
    let mut ctx = TemplateContext::default();
    ctx.set("hints", TemplateValue::List(hints));
    theme.draw(frame, templates::FOOTER, &ctx, area)
}
