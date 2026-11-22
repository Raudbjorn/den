//! Generates den's Oxide palette (Rust source) from IRONMAW's resolved Oxide tokens.
//!
//! Pure: JSON text in, Rust source text out. Shared by `build.rs` and
//! `tests/palette_generation.rs`, so it must only depend on `serde_json` and `std`.

use serde_json::Value;
use std::fmt::{self, Write as _};

/// The `$edition` the token file must declare.
pub const EXPECTED_EDITION: &str = "oxide";

/// One engine palette slot: the `TerminalPalette` field, the IRONMAW colour token
/// that fills it, and its colour in the terminals that can't show the token:
/// an ANSI colour (a `ratatui::style::Color` variant name) for 16 colours and
/// one of the first eight for 8 colours, each chosen by role; and, for 256
/// colours, an xterm index where the nearest one would merge two roles (#128).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub field: &'static str,
    pub token: &'static str,
    pub ansi16: &'static str,
    pub ansi8: &'static str,
    /// `None`: the token colour, quantised to the nearest xterm colour.
    pub xterm256: Option<u8>,
}

const fn slot(
    field: &'static str,
    token: &'static str,
    ansi16: &'static str,
    ansi8: &'static str,
    xterm256: Option<u8>,
) -> Slot {
    Slot {
        field,
        token,
        ansi16,
        ansi8,
        xterm256,
    }
}

/// The colours an 8-colour terminal has: `Slot::ansi8` must be one of them.
pub const ANSI8_COLOURS: [&str; 8] = [
    "Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "Gray",
];

/// Every `TerminalPalette` field, in declaration order. See `assets/tokens/SOURCE.md`.
pub const SLOTS: [Slot; 27] = [
    // 256 colours: the fills are pinned one greyscale step apart (nearest-colour
    // merges page and mark plate, surfaces 1 and 2, surface 3 and selected).
    slot("bg", "bg", "Black", "Black", Some(233)),
    slot("surface_1", "surface-1", "Black", "Black", Some(234)),
    slot("surface_2", "surface-2", "Black", "Black", Some(235)),
    slot("surface_3", "surface-3", "Black", "Black", Some(237)),
    // 8 colours: slate lines in blue, so structure isn't body text. They are
    // never drawn on the selected fill (also blue).
    slot("border", "border", "DarkGray", "Blue", Some(238)),
    slot("text", "text", "Gray", "Gray", None),
    slot("text_strong", "text-strong", "White", "Gray", None),
    slot("text_muted", "text-muted", "Gray", "Gray", None),
    slot("text_faint", "text-faint", "DarkGray", "Gray", None),
    // Teal: actionable.
    slot("accent", "accent", "Cyan", "Cyan", None),
    // No IRONMAW token: structure text (slate, readable).
    slot("accent_2", "structure-text", "Gray", "Gray", None),
    // Rust: the one action, "you are here".
    slot("accent_rust", "action", "Red", "Red", None),
    slot("mix_target", "mix-target", "White", "Gray", None),
    slot("success", "success", "Green", "Green", None),
    // Never rust's colour: magenta in 8 colours, apart from rust hover in 256.
    slot("error", "error", "LightRed", "Magenta", Some(167)),
    // Amber: caution. Never the same ANSI colour as rust.
    slot("warning", "warning", "Yellow", "Yellow", None),
    // No IRONMAW token: structure (slate), used for sutures. It is drawn on
    // the selected fill (the sidebar's icons), so not blue in 8 colours.
    slot("info", "structure", "DarkGray", "Gray", None),
    slot("syn_keyword", "accent", "Cyan", "Cyan", None),
    slot("syn_string", "success", "Green", "Green", None),
    slot("syn_var", "text", "Gray", "Gray", None),
    slot("syn_func", "structure-text", "LightBlue", "Blue", None),
    slot("syn_comment", "text-faint", "DarkGray", "Gray", None),
    slot("syn_number", "action-hover", "LightRed", "Magenta", None),
    // The four IRONMAW roles the upstream engine had no slot for.
    slot("selected", "selected", "Blue", "Blue", Some(236)),
    slot("mark_bg", "mark-bg", "Black", "Black", Some(232)),
    slot("border_strong", "border-strong", "DarkGray", "Blue", None),
    // Rust's own hover: rust in 8 colours, apart from error in 256.
    slot("action_hover", "action-hover", "LightRed", "Red", Some(173)),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenError {
    Json(String),
    WrongEdition {
        found: Option<String>,
    },
    MissingColorGroup,
    MissingToken {
        token: &'static str,
    },
    BadHex {
        token: &'static str,
        value: String,
    },
    /// A slot's 8-colour choice isn't one of [`ANSI8_COLOURS`].
    NotAnsi8 {
        field: &'static str,
        colour: &'static str,
    },
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "token file is not valid JSON: {e}"),
            Self::WrongEdition { found } => write!(
                f,
                "token file `$edition` is {found:?}, expected {EXPECTED_EDITION:?}"
            ),
            Self::MissingColorGroup => write!(f, "token file has no `color` object"),
            Self::MissingToken { token } => {
                write!(f, "token `color.{token}` is missing or not a string")
            }
            Self::BadHex { token, value } => {
                write!(f, "token `color.{token}` = {value:?} is not #RRGGBB")
            }
            Self::NotAnsi8 { field, colour } => write!(
                f,
                "slot `{field}`'s 8-colour choice {colour:?} is not one of {ANSI8_COLOURS:?}"
            ),
        }
    }
}

impl std::error::Error for GenError {}

pub type Rgb = (u8, u8, u8);

/// Parses `#RRGGBB` (case-insensitive). Anything else is `None`.
pub fn parse_hex(value: &str) -> Option<Rgb> {
    const HEX_DIGITS: usize = 6;
    let digits = value.strip_prefix('#')?;
    if digits.len() != HEX_DIGITS || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |start: usize| u8::from_str_radix(digits.get(start..start + 2)?, 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// Resolves every slot's colour from the token file, in [`SLOTS`] order.
pub fn resolve(tokens_json: &str) -> Result<Vec<(Slot, Rgb)>, GenError> {
    let doc: Value =
        serde_json::from_str(tokens_json).map_err(|e| GenError::Json(e.to_string()))?;
    let edition = doc.get("$edition").and_then(Value::as_str);
    if edition != Some(EXPECTED_EDITION) {
        return Err(GenError::WrongEdition {
            found: edition.map(str::to_owned),
        });
    }
    let colors = doc
        .get("color")
        .and_then(Value::as_object)
        .ok_or(GenError::MissingColorGroup)?;

    if let Some(slot) = SLOTS.iter().find(|s| !ANSI8_COLOURS.contains(&s.ansi8)) {
        return Err(GenError::NotAnsi8 {
            field: slot.field,
            colour: slot.ansi8,
        });
    }
    SLOTS
        .iter()
        .map(|&slot| {
            let value = colors
                .get(slot.token)
                .and_then(Value::as_str)
                .ok_or(GenError::MissingToken { token: slot.token })?;
            let rgb = parse_hex(value).ok_or_else(|| GenError::BadHex {
                token: slot.token,
                value: value.to_owned(),
            })?;
            Ok((slot, rgb))
        })
        .collect()
}

/// Renders the generated module: `OXIDE_TRUECOLOR`, `OXIDE_ANSI256`,
/// `OXIDE_ANSI16` and `OXIDE_ANSI8`.
pub fn generate(tokens_json: &str) -> Result<String, GenError> {
    let resolved = resolve(tokens_json)?;
    let mut out = String::new();
    // `write!` into a String cannot fail; the results are ignored deliberately.
    let _ = writeln!(
        out,
        "// @generated by build.rs from assets/tokens/oxide.tokens.json. Do not edit.\n"
    );
    let _ = writeln!(
        out,
        "/// IRONMAW Oxide, exact token colours (truecolor).\n\
         pub const OXIDE_TRUECOLOR: ::raudbjorn_tui::theme::TerminalPalette = ::raudbjorn_tui::theme::TerminalPalette {{"
    );
    for (slot, (r, g, b)) in &resolved {
        let _ = writeln!(
            out,
            "    {}: ::ratatui::style::Color::Rgb({r}, {g}, {b}), // color.{}",
            slot.field, slot.token
        );
    }
    let _ = writeln!(out, "}};\n");
    let _ = writeln!(
        out,
        "/// IRONMAW Oxide for 256 colours: token colours (quantised per cell to the\n\
         /// nearest xterm colour), except the roles that would merge, pinned (#128).\n\
         pub const OXIDE_ANSI256: ::raudbjorn_tui::theme::TerminalPalette = ::raudbjorn_tui::theme::TerminalPalette {{"
    );
    for (slot, (r, g, b)) in &resolved {
        let _ = match slot.xterm256 {
            Some(index) => writeln!(
                out,
                "    {}: ::ratatui::style::Color::Indexed({index}), // pinned",
                slot.field
            ),
            None => writeln!(
                out,
                "    {}: ::ratatui::style::Color::Rgb({r}, {g}, {b}), // color.{}",
                slot.field, slot.token
            ),
        };
    }
    let _ = writeln!(out, "}};\n");
    let _ = writeln!(
        out,
        "/// IRONMAW Oxide by role, for 16-colour terminals.\n\
         pub const OXIDE_ANSI16: ::raudbjorn_tui::theme::TerminalPalette = ::raudbjorn_tui::theme::TerminalPalette {{"
    );
    for (slot, _) in &resolved {
        let _ = writeln!(
            out,
            "    {}: ::ratatui::style::Color::{},",
            slot.field, slot.ansi16
        );
    }
    let _ = writeln!(out, "}};\n");
    let _ = writeln!(
        out,
        "/// IRONMAW Oxide by role, for 8-colour terminals (#128): rust stays the one\n\
         /// action's colour, and slate lines stay apart from text.\n\
         pub const OXIDE_ANSI8: ::raudbjorn_tui::theme::TerminalPalette = ::raudbjorn_tui::theme::TerminalPalette {{"
    );
    for (slot, _) in &resolved {
        let _ = writeln!(
            out,
            "    {}: ::ratatui::style::Color::{},",
            slot.field, slot.ansi8
        );
    }
    let _ = writeln!(out, "}};");
    Ok(out)
}
