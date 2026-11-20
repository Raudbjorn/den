//! The sidebar plate: Scopes (All Owners, each Owner, its Repos), windowed so
//! the cursor (or, unfocused, the current Scope) stays in view.

use super::{
    Theme, UiError,
    glyphs::{self, Glyphs},
    templates,
};
use crate::app::view_model::{Sidebar, SidebarEntry, SidebarRow};
use crepuscularity_tui::TemplateContext;
use ratatui::{Frame, layout::Rect};

/// Lines above the first row: top padding, label, spacer.
const CHROME_LINES: u16 = 3;
/// Lines per sidebar row.
const ROW_LINES: u16 = 1;
/// A Repo's indent under its Owner, in columns.
const REPO_INDENT: &str = "  ";

// The `SidebarRow` template's fixed columns, in cells (its `w-[n]` spans); the
// label takes what is left.
/// The cursor tick, then the current-Scope glyph.
const MARK_COLS: usize = 2;
/// The "upstream →" label of a fork that targets its Upstream.
const UPSTREAM_COLS: usize = 11;
/// The blank before a Repo's fork and archived glyphs.
const ICONS_GAP_COLS: usize = 1;
/// Fork and archived glyphs.
const ICONS_COLS: usize = 2;
/// The incomplete-coverage glyph.
const FLAG_COLS: usize = 1;
/// The count, right-aligned with at least one blank before it, so a 4-digit
/// count stays apart from the incomplete glyph ("⚠1101").
const COUNT_COLS: usize = 5;
/// The blank between the count and the sidebar's rail, so a count as wide as
/// its column never touches the rail.
const RAIL_GAP_COLS: usize = 1;
/// The largest count drawn in full in [`COUNT_COLS`] (four digits and the
/// blank); above it, [`COUNT_OVERFLOW`].
const MAX_EXACT_COUNT: usize = 9_999;
/// Drawn for a count above [`MAX_EXACT_COUNT`]: still true, still four cells.
const COUNT_OVERFLOW: &str = "10k+";

/// Fork and archived glyphs (each also has an ASCII form).
struct RepoGlyphs {
    fork: &'static str,
    /// A fork that targets its Upstream: always this text label, never colour alone.
    upstream: &'static str,
    archived: &'static str,
    current_scope: &'static str,
    none: &'static str,
}

fn repo_glyphs(profile: raudbjorn_tui::profile::GlyphProfile) -> RepoGlyphs {
    match profile {
        raudbjorn_tui::profile::GlyphProfile::Unicode => RepoGlyphs {
            fork: "⑂",
            upstream: "upstream →",
            archived: "▣",
            current_scope: "●",
            none: "·",
        },
        raudbjorn_tui::profile::GlyphProfile::Ascii => RepoGlyphs {
            fork: "Y",
            upstream: "upstream >",
            archived: "#",
            current_scope: "*",
            none: ".",
        },
    }
}

/// Which rows fit, keeping `anchor` in view: `start..start + count`.
pub fn window(total: usize, anchor: Option<usize>, height: u16) -> (usize, usize) {
    let fits = usize::from(height.saturating_sub(CHROME_LINES)).max(1);
    let count = fits.min(total);
    let start = match anchor {
        Some(i) if i >= count => i + 1 - count,
        _ => 0,
    };
    (start, count)
}

pub fn draw(
    frame: &mut Frame,
    theme: &Theme,
    sidebar: &Sidebar,
    area: Rect,
) -> Result<(), UiError> {
    let profile = theme.profile().glyphs;
    let shared = glyphs::for_profile(profile);
    let marks = repo_glyphs(profile);
    let anchor = if sidebar.focused {
        sidebar.cursor
    } else {
        sidebar.rows.iter().position(|r| r.current)
    };
    let repos = sidebar
        .rows
        .iter()
        .filter(|r| r.entry == SidebarEntry::Repo)
        .count();
    let mut ctx = TemplateContext::default();
    ctx.set("sidebar_label", sidebar.label.as_str())
        .set("position", repos.to_string());
    theme.draw(frame, templates::SIDEBAR, &ctx, area)?;
    // One call per visible row, as in `ui::list` (the engine clones its whole
    // context per element).
    let (start, count) = window(sidebar.rows.len(), anchor, area.height);
    let first_row_y = area.y.saturating_add(CHROME_LINES);
    let rows = sidebar.rows.iter().enumerate().skip(start).take(count);
    for (offset, (index, row)) in rows.enumerate() {
        let row_area = super::list::row_area(area, first_row_y, offset, ROW_LINES);
        if row_area.is_empty() {
            break;
        }
        let cursor = sidebar.focused && sidebar.cursor == Some(index);
        let label_cols = label_cols(row, usize::from(row_area.width));
        let ctx = row_context(row, cursor, &shared, &marks, label_cols);
        theme.draw(frame, templates::SIDEBAR_ROW, &ctx, row_area)?;
    }
    Ok(())
}

/// The cells a row's label may take in a row `width` cells wide: what the
/// template's fixed columns for this kind of row leave.
fn label_cols(row: &SidebarRow, width: usize) -> usize {
    let fixed = if row.entry == SidebarEntry::Repo {
        let upstream = if row.upstream.is_some() {
            UPSTREAM_COLS
        } else {
            0
        };
        MARK_COLS + upstream + ICONS_GAP_COLS + ICONS_COLS + FLAG_COLS + COUNT_COLS + RAIL_GAP_COLS
    } else {
        MARK_COLS + FLAG_COLS + COUNT_COLS + RAIL_GAP_COLS
    };
    width.saturating_sub(fixed)
}

/// The count in [`COUNT_COLS`] cells, right-aligned; a faint dot for none.
fn count_text(count: usize, none: &str) -> String {
    let text = match count {
        0 => none.to_owned(),
        n if n > MAX_EXACT_COUNT => COUNT_OVERFLOW.to_owned(),
        n => n.to_string(),
    };
    format!("{text:>COUNT_COLS$}")
}

fn row_context(
    row: &SidebarRow,
    cursor: bool,
    glyphs: &Glyphs,
    marks: &RepoGlyphs,
    label_cols: usize,
) -> TemplateContext {
    let heading = row.entry != SidebarEntry::Repo;
    let label = if heading {
        row.label.clone()
    } else {
        format!("{REPO_INDENT}{}", row.label)
    };
    // Cut here, with an ellipsis: left to the template, a long label wraps at
    // a hyphen or space and loses its end without a sign.
    let label = glyphs.fit(&label, label_cols);
    let mut icons = String::new();
    if row.fork {
        icons.push_str(marks.fork);
    }
    if row.archived {
        icons.push_str(marks.archived);
    }
    let mut ctx = TemplateContext::default();
    ctx.set("cursor", cursor)
        .set("heading", heading)
        .set("current", row.current)
        .set("busy", row.count > 0)
        .set("mark", if cursor { glyphs.tick } else { " " })
        .set(
            "scope_mark",
            if row.current {
                marks.current_scope
            } else {
                " "
            },
        )
        .set("label", label)
        .set("icons", icons)
        .set("has_upstream", row.upstream.is_some())
        .set("upstream", marks.upstream)
        .set(
            "flag",
            if row.incomplete {
                glyphs.incomplete
            } else {
                " "
            },
        )
        .set("count", count_text(row.count, marks.none));
    ctx
}
