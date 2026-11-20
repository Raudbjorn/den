//! IRONMAW Oxide as engine palettes.
//!
//! Generated at build time from the committed token file
//! (`assets/tokens/oxide.tokens.json`, see `assets/tokens/SOURCE.md`). Never edit
//! colours here: change the token file (or the slot table in `build/palette_gen.rs`)
//! and rebuild.

use raudbjorn_tui::{profile::ColorProfile, theme::TerminalPalette};

include!(concat!(env!("OUT_DIR"), "/oxide_palette.rs"));

/// The palette to hand the engine for a colour profile.
///
/// The engine replaces sentinel colours with these roles before it applies the
/// profile, so roles chosen here survive quantisation. 16- and 8-colour terminals
/// get the role-based ANSI palette, because nearest-colour matching would merge
/// roles IRONMAW keeps apart (rust and amber, the selected fill and the page).
pub const fn oxide_for(color: ColorProfile) -> TerminalPalette {
    match color {
        ColorProfile::Ansi16 | ColorProfile::Ansi8 => OXIDE_ANSI16,
        ColorProfile::TrueColor
        | ColorProfile::Ansi256
        | ColorProfile::Mono
        | ColorProfile::NoColor => OXIDE_TRUECOLOR,
    }
}
