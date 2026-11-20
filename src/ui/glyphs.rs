//! Glyphs per glyph profile. Every colour-coded meaning also has one of these.

use crate::app::view_model::{ELLIPSIS, ENTER_KEY, SEPARATOR};
use raudbjorn_tui::profile::GlyphProfile;
use std::borrow::Cow;

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
            separator: "-",
            ellipsis: "...",
            enter: "RET",
        },
    }
}
