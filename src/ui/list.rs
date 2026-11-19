//! The list plate: Work Item rows, windowed so the selection stays in view, or
//! the empty state.

use super::{
    Theme, UiError, glyphs, templates,
    text::{self, LineStyle},
};
use crate::app::view_model::{List, Row};
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{Frame, layout::Rect};

/// Lines per Work Item row.
pub const ROW_LINES: u16 = 2;
/// Lines above the first row: top padding, label, spacer.
const CHROME_LINES: u16 = 3;
/// Horizontal padding of the empty-state text, in columns per side.
const EMPTY_PADDING_COLS: u16 = 4;

pub fn draw(frame: &mut Frame, theme: &Theme, list: &List, area: Rect) -> Result<(), UiError> {
    match &list.empty {
        Some(empty) if list.rows.is_empty() => {
            let width = usize::from(area.width.saturating_sub(2 * EMPTY_PADDING_COLS));
            let max_lines = usize::from(area.height.saturating_sub(CHROME_LINES + 1));
            let lines = text::wrap(&empty.detail, width)
                .into_iter()
                .take(max_lines)
                .map(|line| text::line_context(&line, LineStyle::Muted));
            let mut ctx = TemplateContext::default();
            ctx.set("list_label", list.label.as_str())
                .set("empty_title", empty.title.as_str())
                .set("lines", text::lines_value(lines));
            theme.draw(frame, templates::LIST_EMPTY, &ctx, area)
        }
        _ => draw_rows(frame, theme, list, area),
    }
}

/// Which rows fit, keeping `selected` in view: `start..start + count`.
pub fn window(total: usize, selected: Option<usize>, height: u16) -> (usize, usize) {
    let fits = usize::from(height.saturating_sub(CHROME_LINES) / ROW_LINES).max(1);
    let count = fits.min(total);
    let start = match selected {
        Some(i) if i >= count => i + 1 - count,
        _ => 0,
    };
    (start, count)
}

fn draw_rows(frame: &mut Frame, theme: &Theme, list: &List, area: Rect) -> Result<(), UiError> {
    let glyphs = glyphs::for_profile(theme.profile().glyphs);
    let (start, count) = window(list.rows.len(), list.selected, area.height);
    let rows = list
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(count)
        .map(|(index, row)| row_context(row, list.selected == Some(index), &glyphs))
        .collect();
    let position = match list.selected {
        Some(i) => format!("{} of {}", i + 1, list.rows.len()),
        None => format!("{} items", list.rows.len()),
    };
    let mut ctx = TemplateContext::default();
    ctx.set("list_label", list.label.as_str())
        .set("position", position)
        .set("rows", TemplateValue::List(rows));
    theme.draw(frame, templates::LIST_ROWS, &ctx, area)
}

fn row_context(row: &Row, selected: bool, glyphs: &glyphs::Glyphs) -> TemplateContext {
    let mut flag = String::new();
    if row.incomplete {
        flag.push_str(glyphs.incomplete);
    }
    if row.unattributed {
        flag.push_str(glyphs.unattributed);
    }
    let mut ctx = TemplateContext::default();
    ctx.set("selected", selected)
        .set("dimmed", row.dimmed)
        .set("has_step", row.next_step.is_some())
        .set("tick", if selected { glyphs.tick } else { " " })
        .set("step", row.step_label.as_str())
        .set("flag", flag)
        .set("repo", row.repo.as_str())
        .set("reference", row.reference.as_str())
        .set("title", row.title.as_str())
        .set("waiting", row.waiting_label.as_str())
        .set("age", row.age.as_str());
    ctx
}
