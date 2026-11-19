//! The Tonal-plates frame: header plate, list plate, detail plate, footer, and
//! the sutures and chamfer between them. Draws a ViewModel and nothing else.

use super::{Theme, UiError, detail, footer, glyphs, header, list, templates};
use crate::app::ViewModel;
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
};

/// Height of the header plate, in rows.
pub const HEADER_PLATE_ROWS: u16 = 4;
/// Height of the footer (key hints), in rows.
pub const FOOTER_ROWS: u16 = 1;
/// Width of a vertical suture, in columns.
pub const SUTURE_COLS: u16 = 1;
/// Width of the detail plate, in columns.
pub const DETAIL_PLATE_COLS: u16 = 40;
/// Size of the chamfer on the detail plate's top-left corner, in cells.
const CHAMFER_CELLS: u16 = 1;

/// Where each plate sits for a given terminal area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRegions {
    pub header: Rect,
    pub list: Rect,
    pub suture: Rect,
    pub detail: Rect,
    pub footer: Rect,
}

pub fn regions(area: Rect) -> FrameRegions {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(HEADER_PLATE_ROWS),
        Constraint::Fill(1),
        Constraint::Length(FOOTER_ROWS),
    ])
    .areas(area);
    let [list, suture, detail] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(SUTURE_COLS),
        Constraint::Length(DETAIL_PLATE_COLS),
    ])
    .areas(body);
    FrameRegions {
        header,
        list,
        suture,
        detail,
        footer,
    }
}

/// Draws the ViewModel over the whole terminal.
pub fn draw(frame: &mut Frame, theme: &Theme, view: &ViewModel) -> Result<(), UiError> {
    let r = regions(frame.area());
    header::draw(frame, theme, &view.header, r.header)?;
    list::draw(frame, theme, &view.list, r.list)?;
    draw_suture(frame, theme, r.suture)?;
    detail::draw(frame, theme, &view.detail, r.detail)?;
    footer::draw(frame, theme, &view.footer, r.footer)?;
    if !r.detail.is_empty() {
        let mut ctx = TemplateContext::default();
        ctx.set(
            "chamfer",
            glyphs::for_profile(theme.profile().glyphs).chamfer,
        );
        let corner = Rect::new(r.detail.x, r.detail.y, CHAMFER_CELLS, CHAMFER_CELLS);
        theme.draw(frame, templates::CHAMFER, &ctx, corner)?;
    }
    if let Some(overlay) = &view.overlay {
        match *overlay {}
    }
    Ok(())
}

fn draw_suture(frame: &mut Frame, theme: &Theme, area: Rect) -> Result<(), UiError> {
    let rows = (0..area.height)
        .map(|_| TemplateContext::default())
        .collect();
    let mut ctx = TemplateContext::default();
    ctx.set("rows", TemplateValue::List(rows));
    theme.draw(frame, templates::SUTURE_V, &ctx, area)
}

/// Renders the ViewModel off-screen into a buffer of `width` x `height` cells.
pub fn render(theme: &Theme, view: &ViewModel, width: u16, height: u16) -> Result<Buffer, UiError> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))
        .map_err(|e| UiError::Buffer(std::io::Error::other(e.to_string())))?;
    let mut outcome = Ok(());
    terminal
        .draw(|frame| outcome = draw(frame, theme, view))
        .map_err(|e| UiError::Buffer(std::io::Error::other(e.to_string())))?;
    outcome?;
    Ok(terminal.backend().buffer().clone())
}

/// The text of a rendered buffer, one line per row, trailing spaces trimmed.
/// A debugging aid (`den --dump`), not a test seam.
pub fn buffer_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    (area.top()..area.bottom())
        .map(|y| {
            let row: String = (area.left()..area.right())
                .map(|x| buffer.cell((x, y)).map_or(" ", |cell| cell.symbol()))
                .collect();
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
