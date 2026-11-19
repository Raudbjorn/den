//! The sidebar plate: Scopes (All Owners, each Owner, its Repos), windowed so
//! the cursor (or, unfocused, the current Scope) stays in view.

use super::{Theme, UiError, glyphs, templates};
use crate::app::view_model::{Sidebar, SidebarEntry, SidebarRow};
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{Frame, layout::Rect};

/// Lines above the first row: top padding, label, spacer.
const CHROME_LINES: u16 = 3;
/// A Repo's indent under its Owner, in columns.
const REPO_INDENT: &str = "  ";

/// Fork and archived glyphs (each also has an ASCII form).
struct RepoGlyphs {
    fork: &'static str,
    archived: &'static str,
    current_scope: &'static str,
    none: &'static str,
}

fn repo_glyphs(profile: raudbjorn_tui::profile::GlyphProfile) -> RepoGlyphs {
    match profile {
        raudbjorn_tui::profile::GlyphProfile::Unicode => RepoGlyphs {
            fork: "⑂",
            archived: "▣",
            current_scope: "●",
            none: "·",
        },
        raudbjorn_tui::profile::GlyphProfile::Ascii => RepoGlyphs {
            fork: "Y",
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
    let (start, count) = window(sidebar.rows.len(), anchor, area.height);
    let rows = sidebar
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(count)
        .map(|(index, row)| {
            let cursor = sidebar.focused && sidebar.cursor == Some(index);
            row_context(row, cursor, shared.tick, shared.incomplete, &marks)
        })
        .collect();
    let repos = sidebar
        .rows
        .iter()
        .filter(|r| r.entry == SidebarEntry::Repo)
        .count();
    let mut ctx = TemplateContext::default();
    ctx.set("sidebar_label", sidebar.label.as_str())
        .set("position", repos.to_string())
        .set("rows", TemplateValue::List(rows));
    theme.draw(frame, templates::SIDEBAR, &ctx, area)
}

fn row_context(
    row: &SidebarRow,
    cursor: bool,
    tick: &str,
    incomplete: &str,
    marks: &RepoGlyphs,
) -> TemplateContext {
    let heading = row.entry != SidebarEntry::Repo;
    let label = if heading {
        row.label.clone()
    } else {
        format!("{REPO_INDENT}{}", row.label)
    };
    let mut icons = String::new();
    if row.fork {
        icons.push_str(marks.fork);
    }
    if row.archived {
        icons.push_str(marks.archived);
    }
    let count = match row.count {
        0 => marks.none.to_owned(),
        n => n.to_string(),
    };
    let mut ctx = TemplateContext::default();
    ctx.set("cursor", cursor)
        .set("heading", heading)
        .set("current", row.current)
        .set("busy", row.count > 0)
        .set("mark", if cursor { tick } else { " " })
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
        .set("flag", if row.incomplete { incomplete } else { " " })
        .set("count", format!("{count:>3}"));
    ctx
}
