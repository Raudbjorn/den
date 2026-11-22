//! Glyphs per glyph profile. Colour-coded meanings use these or explicit words.

use crate::app::view_model::{ELLIPSIS, ENTER_KEY, SEPARATOR};
use raudbjorn_tui::profile::GlyphProfile;
use std::borrow::Cow;

#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    /// The rust tick on the current row.
    pub tick: &'static str,
    /// Some source behind an item or view was not read completely.
    /// Also the shared marker for cautions on the launch sheet.
    pub incomplete: &'static str,
    /// Waiting On could not be established.
    pub unattributed: &'static str,
    pub complete: &'static str,
    pub chamfer: &'static str,
    /// The text cursor in an input field.
    pub cursor: &'static str,
    /// A Live Job: its worktree exists and is registered (teal) (#14). Also
    /// a Live Job whose session den couldn't tell running or idle (#80).
    pub live_job: &'static str,
    /// A Live Job whose session has a local `claude` process (teal) (#80).
    pub job_running: &'static str,
    /// A Live Job whose session has no local `claude` process (teal) (#80).
    pub job_idle: &'static str,
    /// A Job whose worktree is gone (faint).
    pub job_missing: &'static str,
    /// A Job whose worktree den can't tell about (amber).
    pub job_unknown: &'static str,
    /// A Linear Issue in the list, and an Owner with a Linear Workflow in the
    /// sidebar and header (#32).
    pub linear: &'static str,
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
    /// `width` cells: whole when it fits, otherwise cut between graphemes and
    /// ended with the profile's form of [`ELLIPSIS`] (see [`super::text::fit`]).
    pub fn fit(&self, text: &str, width: usize) -> String {
        super::text::fit(&self.text(text), width, self.ellipsis)
    }
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
            job_running: "▸",
            job_idle: "◐",
            job_missing: "○",
            job_unknown: "◌",
            linear: "◇",
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
            job_running: "@",
            job_idle: ".",
            job_missing: "o",
            job_unknown: "~",
            linear: "[L]",
            separator: "-",
            ellipsis: "...",
            enter: "RET",
        },
    }
}
