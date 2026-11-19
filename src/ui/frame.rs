//! The empty Tonal-plates frame: header plate, list plate, detail plate, footer.

use super::{Theme, UiError, templates};
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
};
use raudbjorn_tui::profile::GlyphProfile;

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

const BRAND: &str = "DEN";
const TAGLINE: &str = "Next Steps across your Owners";
const NOT_SURVEYED: &str = "Not surveyed yet";
const LIST_LABEL: &str = "N E X T   S T E P S";
const EMPTY_TITLE: &str = "No survey yet.";
const EMPTY_DETAIL: &str = "den has not looked at any Owners.";
const DETAIL_LABEL: &str = "D E T A I L";
const DETAIL_EMPTY: &str = "Nothing selected.";
const CHAMFER_UNICODE: &str = "◢";
const CHAMFER_ASCII: &str = "/";

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

fn frame_context(theme: &Theme, suture_rows: u16) -> TemplateContext {
    let mut ctx = TemplateContext::default();
    let chamfer = match theme.profile().glyphs {
        GlyphProfile::Unicode => CHAMFER_UNICODE,
        GlyphProfile::Ascii => CHAMFER_ASCII,
    };
    let rows = (0..suture_rows)
        .map(|_| TemplateContext::default())
        .collect();
    ctx.set("brand", BRAND)
        .set("tagline", TAGLINE)
        .set("coverage", NOT_SURVEYED)
        .set("list_label", LIST_LABEL)
        .set("empty_title", EMPTY_TITLE)
        .set("empty_detail", EMPTY_DETAIL)
        .set("detail_label", DETAIL_LABEL)
        .set("detail_empty", DETAIL_EMPTY)
        .set("chamfer", chamfer)
        .set("rows", TemplateValue::List(rows));
    ctx
}

/// Draws the empty frame over the whole terminal.
pub fn draw_empty_frame(frame: &mut Frame, theme: &Theme) -> Result<(), UiError> {
    let r = regions(frame.area());
    let ctx = frame_context(theme, r.suture.height);
    theme.draw(frame, templates::HEADER, &ctx, r.header)?;
    theme.draw(frame, templates::LIST_EMPTY, &ctx, r.list)?;
    theme.draw(frame, templates::SUTURE_V, &ctx, r.suture)?;
    theme.draw(frame, templates::DETAIL, &ctx, r.detail)?;
    theme.draw(frame, templates::FOOTER, &ctx, r.footer)?;
    if !r.detail.is_empty() {
        let corner = Rect::new(r.detail.x, r.detail.y, CHAMFER_CELLS, CHAMFER_CELLS);
        theme.draw(frame, templates::CHAMFER, &ctx, corner)?;
    }
    Ok(())
}

/// Renders the empty frame off-screen into a buffer of `width` x `height` cells.
pub fn render_empty_frame(theme: &Theme, width: u16, height: u16) -> Result<Buffer, UiError> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))
        .map_err(|e| UiError::Buffer(std::io::Error::other(e.to_string())))?;
    let mut outcome = Ok(());
    terminal
        .draw(|frame| outcome = draw_empty_frame(frame, theme))
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
