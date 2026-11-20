//! Glyphs per glyph profile. Every colour-coded meaning also has one of these.

use crate::app::view_model::{ELLIPSIS, ENTER_KEY, SEPARATOR};
use raudbjorn_tui::profile::GlyphProfile;
use std::borrow::Cow;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    /// The rust tick on the current row.
    pub tick: &'static str,
    /// Some source behind an item or view was not read completely.
    pub incomplete: &'static str,
    /// Waiting On could not be established.
    pub unattributed: &'static str,
    pub complete: &'static str,
    pub chamfer: &'static str,
    /// The text cursor in an input field.
    pub cursor: &'static str,
    /// A Live Job: its worktree exists and is registered (teal) (#14).
    pub live_job: &'static str,
    /// A Job whose worktree is gone (faint).
    pub job_missing: &'static str,
    /// A Job whose worktree den can't tell about (amber).
    pub job_unknown: &'static str,
    /// The ViewModel's [`SEPARATOR`] in this profile.
    pub separator: &'static str,
    /// The ViewModel's [`ELLIPSIS`] in this profile.
    pub ellipsis: &'static str,
    /// The ViewModel's [`ENTER_KEY`] in this profile.
    pub enter: &'static str,
}

impl Glyphs {
    /// ViewModel text with its separator, ellipsis and Enter key drawn in this
    /// profile's form (in ASCII the engine would otherwise draw each as `?`).
    pub fn text<'t>(&self, text: &'t str) -> Cow<'t, str> {
        if !text.contains([SEPARATOR, ELLIPSIS, ENTER_KEY]) {
            return Cow::Borrowed(text);
        }
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                SEPARATOR => out.push_str(self.separator),
                ELLIPSIS => out.push_str(self.ellipsis),
                ENTER_KEY => out.push_str(self.enter),
                other => out.push(other),
            }
        }
        Cow::Owned(out)
    }

    /// ViewModel text drawn in this profile (see [`Self::text`]) in at most
    /// `width` cells: whole when it fits, otherwise cut by cell width and
    /// ended with the profile's form of [`ELLIPSIS`], so a cut is never silent.
    /// Narrower than the ellipsis itself, it is as much of the ellipsis as fits.
    pub fn fit(&self, text: &str, width: usize) -> String {
        let drawn = self.text(text);
        if drawn.width() <= width {
            return drawn.into_owned();
        }
        let ellipsis = self.ellipsis;
        let room = width.saturating_sub(ellipsis.width());
        let mut out = take_cells(&drawn, room);
        out.push_str(&take_cells(ellipsis, width.saturating_sub(out.width())));
        out
    }
}

/// The longest prefix of `text` at most `width` cells wide (a wide character
/// that would straddle the edge is left out).
fn take_cells(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut cells = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if cells + w > width {
            break;
        }
        out.push(c);
        cells += w;
    }
    out
}

pub fn for_profile(profile: GlyphProfile) -> Glyphs {
    match profile {
        GlyphProfile::Unicode => Glyphs {
            tick: "▌",
            incomplete: "⚠",
            unattributed: "?",
            complete: "✓",
            chamfer: "◢",
            cursor: "▏",
            live_job: "●",
            job_missing: "○",
            job_unknown: "◌",
            separator: "·",
            ellipsis: "…",
            enter: "⏎",
        },
        GlyphProfile::Ascii => Glyphs {
            tick: ">",
            incomplete: "!",
            unattributed: "?",
            complete: "+",
            chamfer: "/",
            cursor: "|",
            live_job: "*",
            job_missing: "o",
            job_unknown: "~",
            separator: "-",
            ellipsis: "...",
            enter: "RET",
        },
    }
}
