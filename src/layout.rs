//! The responsive layout (spec, "Look and layout"): which plates exist at a given
//! terminal size, and how wide they are. Pure: the App puts the result in the
//! ViewModel, and `ui::frame` turns it into rectangles.
//!
//! Designed for about [`DESIGN_COLS`] columns; usable down to
//! [`MIN_COLS`]×[`MIN_ROWS`]. Below [`WIDE_LAYOUT_MIN_COLS`] the detail pane is
//! hidden and the detail opens as a sheet instead, so the list keeps its width.

/// The width den is designed for, in columns.
pub const DESIGN_COLS: u16 = 160;
/// The height den assumes before the terminal reports its size, in rows.
pub const DESIGN_ROWS: u16 = 45;
/// The smallest supported terminal width, in columns.
pub const MIN_COLS: u16 = 80;
/// The smallest supported terminal height, in rows.
pub const MIN_ROWS: u16 = 24;
/// From this width up, the detail is a pane beside the list; below it, a sheet.
pub const WIDE_LAYOUT_MIN_COLS: u16 = 110;
/// The sidebar's share of the terminal width, in percent.
pub const RAIL_SHARE_PERCENT: u16 = 22;
/// The narrowest the sidebar gets, in columns.
pub const RAIL_MIN_COLS: u16 = 30;
/// The widest the sidebar gets, in columns.
pub const RAIL_MAX_COLS: u16 = 40;
/// The detail pane's share of the terminal width, in percent (40 columns at 160).
pub const DETAIL_SHARE_PERCENT: u16 = 25;
/// The narrowest the detail pane gets, in columns.
pub const DETAIL_MIN_COLS: u16 = 32;
/// The widest the detail pane gets, in columns.
pub const DETAIL_MAX_COLS: u16 = 48;
/// The share of a whole (for the percentages above).
const PERCENT_WHOLE: u32 = 100;

/// The terminal's size, in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    /// The size assumed until the terminal reports one.
    pub const DESIGN: Self = Self {
        cols: DESIGN_COLS,
        rows: DESIGN_ROWS,
    };
}

/// Which plates exist and how wide they are, for one terminal size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub size: TerminalSize,
    /// Width of the sidebar, in columns.
    pub rail_cols: u16,
    /// Width of the detail pane, in columns; `None` when the terminal is too
    /// narrow for one and the detail opens as a sheet instead.
    pub detail_pane_cols: Option<u16>,
    /// The terminal is smaller than [`MIN_COLS`]×[`MIN_ROWS`]; den still draws,
    /// but says so.
    pub undersized: bool,
}

impl Layout {
    pub fn for_size(size: TerminalSize) -> Self {
        Self {
            size,
            rail_cols: share_of(size.cols, RAIL_SHARE_PERCENT).clamp(RAIL_MIN_COLS, RAIL_MAX_COLS),
            detail_pane_cols: (size.cols >= WIDE_LAYOUT_MIN_COLS).then(|| {
                share_of(size.cols, DETAIL_SHARE_PERCENT).clamp(DETAIL_MIN_COLS, DETAIL_MAX_COLS)
            }),
            undersized: size.cols < MIN_COLS || size.rows < MIN_ROWS,
        }
    }

    /// Whether the detail is a pane beside the list (else it is a sheet).
    pub fn has_detail_pane(&self) -> bool {
        self.detail_pane_cols.is_some()
    }
}

/// `percent` of `cols`, rounded down.
fn share_of(cols: u16, percent: u16) -> u16 {
    let share = u32::from(cols) * u32::from(percent) / PERCENT_WHOLE;
    u16::try_from(share).unwrap_or(u16::MAX)
}
