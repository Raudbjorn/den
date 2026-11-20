//! The Tonal-plates frame: header plate (with the bear), sidebar, list plate,
//! detail plate or sheet, footer, and the sutures and chamfer between them.
//! Draws a ViewModel and nothing else.
//!
//! Which plates exist and how wide they are comes from `view.layout`
//! ([`crate::layout`]); this module only turns that into rectangles. Order of
//! drawing: plates, then bears (each skipped if an overlay covers it), then
//! overlays, so a sheet never has bear pixels over it.

use super::{
    Theme, UiError,
    bear::{BearArt, EMPTY_BEAR_MAX_ROWS, EMPTY_BEAR_MIN_ROWS, Placement, SPLASH_BEAR_MAX_ROWS},
    detail, footer, glyphs, header, launch, list, sidebar, templates,
};
use crate::{
    app::{ViewModel, view_model::Overlay},
    layout::{Layout, MIN_COLS, MIN_ROWS},
    palette::OXIDE_TRUECOLOR,
};
use crepuscularity_tui::TemplateContext;
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::{Constraint, Layout as Split, Rect},
    widgets::Clear,
};

/// Height of the header plate, in rows.
pub const HEADER_PLATE_ROWS: u16 = 4;
/// Height of the footer (key hints), in rows.
pub const FOOTER_ROWS: u16 = 1;
/// Width of a vertical suture, in columns.
pub const SUTURE_COLS: u16 = 1;
/// The detail sheet's width at most, in columns.
pub const DETAIL_SHEET_MAX_COLS: u16 = 60;
/// Columns of the screen left uncovered beside the detail sheet, at least.
const DETAIL_SHEET_MIN_UNCOVERED_COLS: u16 = 4;
/// The launch sheet's width at most, in columns.
pub const LAUNCH_SHEET_MAX_COLS: u16 = 110;
/// Columns left uncovered on each side of the launch sheet, at least.
const LAUNCH_SHEET_MARGIN_COLS: u16 = 2;
/// Size of the chamfer on the detail plate's top-left corner, in cells.
const CHAMFER_CELLS: u16 = 1;
/// Columns between the header plate's left edge and the bear.
const HEADER_BEAR_INSET_COLS: u16 = 1;
/// Columns between the header bear and the header text.
const HEADER_BEAR_GAP_COLS: u16 = 1;
/// Rows at the bottom of the splash for the brand, status and margin
/// (the `Splash` template's last five rows, plus one clear row above them).
const SPLASH_TEXT_ROWS: u16 = 6;
/// Rows kept clear above and below the empty-state bear.
const EMPTY_BEAR_MARGIN_ROWS: u16 = 1;
/// Rows of the list plate above its content: top padding and label.
const LIST_LABEL_ROWS: u16 = 2;
const BRAND: &str = "DEN";

/// Where each plate sits for a given terminal area and layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRegions {
    pub header: Rect,
    pub sidebar: Rect,
    /// Between the sidebar and the list.
    pub sidebar_suture: Rect,
    pub list: Rect,
    /// Between the list and the detail pane; empty when there is no pane.
    pub suture: Rect,
    /// The detail pane; empty when the layout has none (the detail is a sheet).
    pub detail: Rect,
    pub footer: Rect,
    /// Everything between the header and the footer.
    pub body: Rect,
}

pub fn regions(area: Rect, layout: &Layout) -> FrameRegions {
    let [header, body, footer] = Split::vertical([
        Constraint::Length(HEADER_PLATE_ROWS),
        Constraint::Fill(1),
        Constraint::Length(FOOTER_ROWS),
    ])
    .areas(area);
    let detail_cols = layout.detail_pane_cols.unwrap_or(0);
    let detail_suture_cols = if detail_cols > 0 { SUTURE_COLS } else { 0 };
    let [sidebar, sidebar_suture, list, suture, detail] = Split::horizontal([
        Constraint::Length(layout.rail_cols),
        Constraint::Length(SUTURE_COLS),
        Constraint::Fill(1),
        Constraint::Length(detail_suture_cols),
        Constraint::Length(detail_cols),
    ])
    .areas(body);
    FrameRegions {
        header,
        sidebar,
        sidebar_suture,
        list,
        suture,
        detail,
        footer,
        body,
    }
}

/// Where the open overlay sits, if any. A bear placement that intersects it is
/// not drawn at all.
pub fn overlay_area(view: &ViewModel, area: Rect) -> Option<Rect> {
    let body = regions(area, &view.layout).body;
    view.overlay.as_ref().map(|overlay| match overlay {
        Overlay::DetailSheet => detail_sheet_area(body),
        Overlay::LaunchSheet(_) => launch_sheet_area(area),
    })
}

/// The launch sheet: centred over the header and body (the footer shows its keys).
fn launch_sheet_area(area: Rect) -> Rect {
    let cols = LAUNCH_SHEET_MAX_COLS.min(area.width.saturating_sub(2 * LAUNCH_SHEET_MARGIN_COLS));
    let rows = area.height.saturating_sub(FOOTER_ROWS);
    Rect::new(area.x + (area.width - cols) / 2, area.y, cols, rows)
}

/// The detail sheet: right-aligned over the body, full body height.
fn detail_sheet_area(body: Rect) -> Rect {
    let cols =
        DETAIL_SHEET_MAX_COLS.min(body.width.saturating_sub(DETAIL_SHEET_MIN_UNCOVERED_COLS));
    Rect::new(body.right() - cols, body.y, cols, body.height)
}

/// Layout-cache entries one terminal row can need per frame: a sidebar row
/// and half a two-line list row each solve a few layouts, plus detail lines.
const LAYOUT_CACHE_ENTRIES_PER_ROW: usize = 8;
/// What ratatui starts with; den never shrinks below it.
const LAYOUT_CACHE_BASE_ENTRIES: usize = Split::DEFAULT_CACHE_SIZE;

thread_local! {
    /// The layout-cache size den last set on this thread.
    static LAYOUT_CACHE_ENTRIES: std::cell::Cell<usize> =
        const { std::cell::Cell::new(LAYOUT_CACHE_BASE_ENTRIES) };
}

/// Grows ratatui's per-thread layout cache to hold one whole frame of `rows`.
///
/// Each list and sidebar row is drawn with its own small layouts, keyed by the
/// row's position. When a frame needs more layouts than the cache holds (500
/// by default), the LRU evicts entries the same frame needs again, so every
/// frame re-solves every row: a 160x400 frame of real data took about 58 ms
/// that way and about 21 ms with room for the whole frame (release build).
fn ensure_layout_cache(rows: u16) {
    let wanted = LAYOUT_CACHE_BASE_ENTRIES
        .saturating_add(usize::from(rows).saturating_mul(LAYOUT_CACHE_ENTRIES_PER_ROW));
    LAYOUT_CACHE_ENTRIES.with(|current| {
        if wanted > current.get()
            && let Some(size) = std::num::NonZeroUsize::new(wanted)
        {
            Split::init_cache(size);
            current.set(wanted);
        }
    });
}

/// Draws the ViewModel over the whole terminal.
pub fn draw(
    frame: &mut Frame,
    theme: &Theme,
    view: &ViewModel,
    bear: &mut BearArt,
) -> Result<(), UiError> {
    ensure_layout_cache(frame.area().height);
    if view.splash {
        return draw_splash(frame, theme, view, bear);
    }
    let area = frame.area();
    let r = regions(area, &view.layout);
    let overlay = overlay_area(view, area);

    let header_bear = header_bear_area(r.header, bear);
    let header_text = Rect {
        x: header_bear.right() + HEADER_BEAR_GAP_COLS,
        width: r
            .header
            .width
            .saturating_sub(header_bear.right() + HEADER_BEAR_GAP_COLS - r.header.x),
        ..r.header
    };
    theme.draw(
        frame,
        templates::MARK_PLATE,
        &TemplateContext::default(),
        r.header,
    )?;
    header::draw(frame, theme, &view.header, header_text)?;
    if !r.sidebar.is_empty() {
        draw_sidebar(frame, theme, view, r.sidebar)?;
        draw_suture(frame, theme, r.sidebar_suture)?;
    }
    list::draw(frame, theme, &view.list, r.list)?;
    if !r.detail.is_empty() {
        draw_suture(frame, theme, r.suture)?;
        detail::draw(frame, theme, &view.detail, r.detail)?;
        draw_chamfer(frame, theme, r.detail)?;
    }
    if view.layout.undersized {
        draw_undersized_notice(frame, theme, &view.layout, r.footer)?;
    } else {
        footer::draw(frame, theme, &view.footer, r.footer)?;
    }

    let uncovered = |b: Rect| !b.is_empty() && !overlay.is_some_and(|o| o.intersects(b));
    if uncovered(header_bear) {
        bear.draw(
            frame,
            Placement::Header,
            header_bear,
            OXIDE_TRUECOLOR.mark_bg,
        );
    }
    let empty_bear = empty_bear_area(view, r.list, bear);
    if uncovered(empty_bear) {
        bear.draw(frame, Placement::Empty, empty_bear, OXIDE_TRUECOLOR.bg);
    }

    if let Some(overlay) = &view.overlay {
        match overlay {
            Overlay::DetailSheet => draw_detail_sheet(frame, theme, view, r.body)?,
            Overlay::LaunchSheet(sheet) => draw_launch_sheet(frame, theme, sheet, area)?,
        }
    }
    Ok(())
}

/// The bear's cell in the header plate: as tall as the plate, left-inset.
fn header_bear_area(header: Rect, bear: &BearArt) -> Rect {
    let cols = bear.cols_for_rows(header.height);
    if cols == 0 {
        return Rect::new(header.x, header.y, 0, header.height);
    }
    let x = header.x + HEADER_BEAR_INSET_COLS;
    Rect::new(
        x,
        header.y,
        cols.min(header.width.saturating_sub(HEADER_BEAR_INSET_COLS)),
        header.height,
    )
}

/// The large bear above the empty state's text, when there is room for it.
fn empty_bear_area(view: &ViewModel, list_area: Rect, bear: &BearArt) -> Rect {
    let Some(text_top) = list::empty_text_top(&view.list, list_area) else {
        return Rect::default();
    };
    let top = list_area.y + LIST_LABEL_ROWS + EMPTY_BEAR_MARGIN_ROWS;
    let bottom = text_top.saturating_sub(EMPTY_BEAR_MARGIN_ROWS);
    let zone = Rect::new(
        list_area.x,
        top,
        list_area.width,
        bottom.saturating_sub(top),
    );
    if zone.height < EMPTY_BEAR_MIN_ROWS {
        return Rect::default();
    }
    bear.fit(zone, EMPTY_BEAR_MAX_ROWS)
}

/// The splash: mark plate, large bear, brand and what den is doing.
fn draw_splash(
    frame: &mut Frame,
    theme: &Theme,
    view: &ViewModel,
    bear: &mut BearArt,
) -> Result<(), UiError> {
    let [plate, footer_area] =
        Split::vertical([Constraint::Fill(1), Constraint::Length(FOOTER_ROWS)]).areas(frame.area());
    let (title, detail) = match &view.list.empty {
        Some(empty) => (empty.title.as_str(), empty.detail.as_str()),
        None => (view.header.status.as_str(), ""),
    };
    let mut ctx = TemplateContext::default();
    ctx.set("brand", BRAND)
        .set("title", title)
        .set("detail", detail);
    theme.draw(frame, templates::SPLASH, &ctx, plate)?;
    footer::draw(frame, theme, &view.footer, footer_area)?;
    let zone = Rect {
        height: plate.height.saturating_sub(SPLASH_TEXT_ROWS),
        ..plate
    };
    let splash_bear = bear.fit(zone, SPLASH_BEAR_MAX_ROWS);
    bear.draw(
        frame,
        Placement::Splash,
        splash_bear,
        OXIDE_TRUECOLOR.mark_bg,
    );
    Ok(())
}

fn draw_detail_sheet(
    frame: &mut Frame,
    theme: &Theme,
    view: &ViewModel,
    body: Rect,
) -> Result<(), UiError> {
    let sheet = detail_sheet_area(body);
    frame.render_widget(Clear, sheet);
    let [suture, plate] =
        Split::horizontal([Constraint::Length(SUTURE_COLS), Constraint::Fill(1)]).areas(sheet);
    draw_suture(frame, theme, suture)?;
    detail::draw(frame, theme, &view.detail, plate)?;
    draw_chamfer(frame, theme, plate)
}

/// The launch sheet: a plate between two sutures, with the chamfer.
fn draw_launch_sheet(
    frame: &mut Frame,
    theme: &Theme,
    sheet: &crate::app::view_model::LaunchSheet,
    area: Rect,
) -> Result<(), UiError> {
    let sheet_area = launch_sheet_area(area);
    frame.render_widget(Clear, sheet_area);
    let [left, plate, right] = Split::horizontal([
        Constraint::Length(SUTURE_COLS),
        Constraint::Fill(1),
        Constraint::Length(SUTURE_COLS),
    ])
    .areas(sheet_area);
    draw_suture(frame, theme, left)?;
    draw_suture(frame, theme, right)?;
    launch::draw(frame, theme, sheet, plate)?;
    draw_chamfer(frame, theme, plate)
}

/// The one chamfer, on a detail plate's top-left corner.
fn draw_chamfer(frame: &mut Frame, theme: &Theme, plate: Rect) -> Result<(), UiError> {
    if plate.is_empty() {
        return Ok(());
    }
    let mut ctx = TemplateContext::default();
    ctx.set(
        "chamfer",
        glyphs::for_profile(theme.profile().glyphs).chamfer,
    );
    let corner = Rect::new(plate.x, plate.y, CHAMFER_CELLS, CHAMFER_CELLS);
    theme.draw(frame, templates::CHAMFER, &ctx, corner)
}

/// A vertical suture: the template draws its top row, which is copied down the
/// rest of the column. (A template row per terminal row made one flex column
/// as tall as the terminal, whose layout solve grew with the square of it.)
fn draw_suture(frame: &mut Frame, theme: &Theme, area: Rect) -> Result<(), UiError> {
    if area.is_empty() {
        return Ok(());
    }
    let top = Rect { height: 1, ..area };
    theme.draw(frame, templates::SUTURE_V, &TemplateContext::default(), top)?;
    let buffer = frame.buffer_mut();
    for x in area.left()..area.right() {
        let Some(cell) = buffer.cell((x, area.y)).cloned() else {
            continue;
        };
        for y in area.top().saturating_add(1)..area.bottom() {
            if let Some(target) = buffer.cell_mut((x, y)) {
                *target = cell.clone();
            }
        }
    }
    Ok(())
}

/// Says the terminal is smaller than den supports, in place of the key hints.
fn draw_undersized_notice(
    frame: &mut Frame,
    theme: &Theme,
    layout: &Layout,
    area: Rect,
) -> Result<(), UiError> {
    let mut ctx = TemplateContext::default();
    ctx.set(
        "glyph",
        glyphs::for_profile(theme.profile().glyphs).incomplete,
    )
    .set(
        "text",
        format!(
            "den needs at least {MIN_COLS}x{MIN_ROWS}; this terminal is {}x{}",
            layout.size.cols, layout.size.rows
        ),
    );
    theme.draw(frame, templates::NOTICE, &ctx, area)
}

/// Renders the ViewModel off-screen into a buffer of `width` x `height` cells,
/// with the bear in half-blocks (or none, if the profile has too few colours).
pub fn render(theme: &Theme, view: &ViewModel, width: u16, height: u16) -> Result<Buffer, UiError> {
    let mut bear = BearArt::offscreen(theme.profile());
    let mut terminal = Terminal::new(TestBackend::new(width, height))
        .map_err(|e| UiError::Buffer(std::io::Error::other(e.to_string())))?;
    let mut outcome = Ok(());
    terminal
        .draw(|frame| outcome = draw(frame, theme, view, &mut bear))
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

/// The sidebar plate (#9 owns its contents; #10 owns where it sits).
fn draw_sidebar(
    frame: &mut Frame,
    theme: &Theme,
    view: &ViewModel,
    area: Rect,
) -> Result<(), UiError> {
    sidebar::draw(frame, theme, &view.sidebar, area)
}
