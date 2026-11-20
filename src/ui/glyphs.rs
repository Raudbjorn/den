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
    /// A Live Job: its worktree exists and is registered (teal) (#14).
    pub live_job: &'static str,
    /// A Job whose worktree is gone (faint).
    pub job_missing: &'static str,
    /// A Job whose worktree den can't tell about (amber).
    pub job_unknown: &'static str,
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
        },
    }
}
