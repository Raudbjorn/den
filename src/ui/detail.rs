//! The detail plate: the selected item's Next Step, Waiting On and Explanation.

use super::{
    Theme, UiError, glyphs, templates,
    text::{self, LineStyle},
};
use crate::app::view_model::Detail;
use crepuscularity_tui::TemplateContext;
use ratatui::{Frame, layout::Rect};

/// Columns lost to the plate's horizontal padding.
const PADDING_COLS: u16 = 4;
/// Rows above the first line: top padding, label, spacer.
const CHROME_ROWS: u16 = 3;
/// The most lines a title may take.
const TITLE_MAX_LINES: usize = 3;

pub fn draw(frame: &mut Frame, theme: &Theme, detail: &Detail, area: Rect) -> Result<(), UiError> {
    let width = usize::from(area.width.saturating_sub(PADDING_COLS));
    let height = usize::from(area.height.saturating_sub(CHROME_ROWS));
    let glyphs = glyphs::for_profile(theme.profile().glyphs);
    let mut lines: Vec<(String, LineStyle)> = Vec::new();
    match &detail.content {
        None => lines.extend(styled(&detail.placeholder, width, LineStyle::Muted)),
        Some(content) => {
            lines.extend(styled(&content.title, width, LineStyle::Strong).take(TITLE_MAX_LINES));
            let mut push = |text: &str, style: LineStyle| lines.extend(styled(text, width, style));
            push(&content.reference, LineStyle::Muted);
            push(" ", LineStyle::Plain);
            push("NEXT STEP", LineStyle::Label);
            push(&content.next_step, LineStyle::Step);
            push("WAITING ON", LineStyle::Label);
            push(&content.waiting_on, LineStyle::Plain);
            push(&content.waiting_reason, LineStyle::Muted);
            push("WHY", LineStyle::Label);
            push(&content.why, LineStyle::Plain);
            if !content.also.is_empty() {
                push("ALSO", LineStyle::Label);
                for also in &content.also {
                    push(also, LineStyle::Plain);
                }
            }
            if !content.coverage.is_empty() {
                push("INCOMPLETE DATA", LineStyle::Label);
                for note in &content.coverage {
                    push(&format!("{} {note}", glyphs.incomplete), LineStyle::Caution);
                }
            }
            if !content.unknowns.is_empty() {
                push("UNKNOWN", LineStyle::Label);
                for unknown in &content.unknowns {
                    push(&format!("? {unknown}"), LineStyle::Muted);
                }
            }
            push("OBSERVED", LineStyle::Label);
            push(&content.observed, LineStyle::Muted);
            push(" ", LineStyle::Plain);
            push(&content.url, LineStyle::Muted);
        }
    }
    let lines = lines
        .into_iter()
        .take(height)
        .map(|(text, style)| text::line_context(&text, style));
    let mut ctx = TemplateContext::default();
    ctx.set("detail_label", detail.label.as_str())
        .set("lines", text::lines_value(lines));
    theme.draw(frame, templates::DETAIL, &ctx, area)
}

fn styled(text: &str, width: usize, style: LineStyle) -> impl Iterator<Item = (String, LineStyle)> {
    text::wrap(text, width)
        .into_iter()
        .map(move |line| (line, style))
}
