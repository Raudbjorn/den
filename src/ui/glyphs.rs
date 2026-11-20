//! Glyphs per glyph profile. Every colour-coded meaning also has one of these.

use raudbjorn_tui::profile::GlyphProfile;

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
        },
        GlyphProfile::Ascii => Glyphs {
            tick: ">",
            incomplete: "!",
            unattributed: "?",
            complete: "+",
            chamfer: "/",
            cursor: "|",
        },
    }
}
