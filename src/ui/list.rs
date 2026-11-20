//! The list plate: Work Item rows, windowed so the selection stays in view, or
//! the empty state.

use super::{
    Theme, UiError,
    glyphs::{self, Glyphs},
    templates,
    text::{self, LineStyle},
};
use crate::{
    app::view_model::{JobMarker, List, Row},
    domain::WorkItemKind,
};
use crepuscularity_tui::TemplateContext;
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
            let glyphs = glyphs::for_profile(theme.profile().glyphs);
            let lines = empty_detail_lines(&empty.detail, area, &glyphs)
                .into_iter()
                .map(|line| text::line_context(&line, LineStyle::Muted));
            let mut ctx = TemplateContext::default();
            ctx.set("list_label", list.label.as_str())
                .set("empty_title", glyphs.text(&empty.title).as_ref())
                .set("lines", text::lines_value(lines));
            theme.draw(frame, templates::LIST_EMPTY, &ctx, area)
        }
        _ => draw_rows(frame, theme, list, area),
    }
}

/// The empty state's detail in the profile's glyphs, wrapped to the plate and
/// cut to what fits.
fn empty_detail_lines(detail: &str, area: Rect, glyphs: &Glyphs) -> Vec<String> {
    let width = usize::from(area.width.saturating_sub(2 * EMPTY_PADDING_COLS));
    let max_lines = usize::from(area.height.saturating_sub(CHROME_LINES + 1));
    text::wrap(&glyphs.text(detail), width)
        .into_iter()
        .take(max_lines)
        .collect()
}

/// Where the empty-state text block (title and detail lines) starts, when the
/// empty state is shown: the `Empty` template centres it below the label. The
/// frame puts the large bear in the space above it (#10).
pub(super) fn empty_text_top(list: &List, area: Rect, glyphs: &Glyphs) -> Option<u16> {
    let empty = list.empty.as_ref().filter(|_| list.rows.is_empty())?;
    let block = u16::try_from(empty_detail_lines(&empty.detail, area, glyphs).len())
        .unwrap_or(u16::MAX)
        .saturating_add(1);
    let label_rows = CHROME_LINES - 1;
    let free = area.height.saturating_sub(label_rows).saturating_sub(block);
    Some(area.y + label_rows + free / 2)
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

/// The plate (label and position), then each visible row in its own call: the
/// template engine clones its whole context per element, so handing it every
/// row at once made a frame cost the square of the rows on screen.
fn draw_rows(frame: &mut Frame, theme: &Theme, list: &List, area: Rect) -> Result<(), UiError> {
    let glyphs = glyphs::for_profile(theme.profile().glyphs);
    let position = match list.selected {
        Some(i) => format!("{} of {}", i + 1, list.rows.len()),
        None => format!("{} items", list.rows.len()),
    };
    let mut ctx = TemplateContext::default();
    ctx.set("list_label", list.label.as_str())
        .set("position", position);
    theme.draw(frame, templates::LIST_ROWS, &ctx, area)?;
    let (start, count) = window(list.rows.len(), list.selected, area.height);
    let first_row_y = area.y.saturating_add(CHROME_LINES);
    for (offset, (index, row)) in list
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(count)
        .enumerate()
    {
        let row_area = row_area(area, first_row_y, offset, ROW_LINES);
        if row_area.is_empty() {
            break;
        }
        let ctx = row_context(row, list.selected == Some(index), &glyphs);
        theme.draw(frame, templates::LIST_ROW, &ctx, row_area)?;
    }
    Ok(())
}

/// The `offset`th row of `lines` lines below `first_y`, clipped to `area`.
pub(super) fn row_area(area: Rect, first_y: u16, offset: usize, lines: u16) -> Rect {
    let offset = u16::try_from(offset).unwrap_or(u16::MAX);
    let y = first_y.saturating_add(offset.saturating_mul(lines));
    let height = lines.min(area.bottom().saturating_sub(y));
    Rect::new(area.x, y, area.width, height)
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
        .set("repo", place(row, glyphs))
        .set("reference", row.reference.as_str())
        .set("title", row.title.as_str())
        .set("waiting", row.waiting_label.as_str())
        .set("age", row.age.as_str())
        .set("job", job_glyph(row.job, glyphs))
        .set("job_live", row.job == Some(JobMarker::Live))
        .set("job_unknown", row.job == Some(JobMarker::Unknown));
    ctx
}

/// The Job marker before the title (#14): a glyph, coloured by the template.
pub fn job_glyph(job: Option<JobMarker>, glyphs: &glyphs::Glyphs) -> &'static str {
    match job {
        Some(JobMarker::Live) => glyphs.live_job,
        Some(JobMarker::Missing) => glyphs.job_missing,
        Some(JobMarker::Unknown) => glyphs.job_unknown,
        None => " ",
    }
}

/// The row's Repo, or for a Linear Issue the Linear glyph and its Owner: the
/// glyph, not colour, says it is Linear's (#32).
fn place(row: &Row, glyphs: &glyphs::Glyphs) -> String {
    if row.kind == WorkItemKind::LinearIssue {
        format!("{} {}", glyphs.linear, row.repo)
    } else {
        row.repo.clone()
    }
}
