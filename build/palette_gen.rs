//! Generates den's Oxide palette (Rust source) from IRONMAW's resolved Oxide tokens.
//!
//! Pure: JSON text in, Rust source text out. Shared by `build.rs` and
//! `tests/palette_generation.rs`, so it must only depend on `serde_json` and `std`.

use serde_json::Value;
use std::fmt::{self, Write as _};

/// The `$edition` the token file must declare.
pub const EXPECTED_EDITION: &str = "oxide";

/// One engine palette slot: the `TerminalPalette` field, the IRONMAW colour token
/// that fills it, and the ANSI colour (a `ratatui::style::Color` variant name)
/// used in 16- and 8-colour terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub field: &'static str,
    pub token: &'static str,
    pub ansi16: &'static str,
}

const fn slot(field: &'static str, token: &'static str, ansi16: &'static str) -> Slot {
    Slot {
        field,
        token,
        ansi16,
    }
}

/// Every `TerminalPalette` field, in declaration order. See `assets/tokens/SOURCE.md`.
pub const SLOTS: [Slot; 27] = [
    slot("bg", "bg", "Black"),
    slot("surface_1", "surface-1", "Black"),
    slot("surface_2", "surface-2", "Black"),
    slot("surface_3", "surface-3", "Black"),
    slot("border", "border", "DarkGray"),
    slot("text", "text", "Gray"),
    slot("text_strong", "text-strong", "White"),
    slot("text_muted", "text-muted", "Gray"),
    slot("text_faint", "text-faint", "DarkGray"),
    // Teal: actionable.
    slot("accent", "accent", "Cyan"),
    // No IRONMAW token: structure text (slate, readable).
    slot("accent_2", "structure-text", "Gray"),
    // Rust: the one action, "you are here".
    slot("accent_rust", "action", "Red"),
    slot("mix_target", "mix-target", "White"),
    slot("success", "success", "Green"),
    slot("error", "error", "LightRed"),
    // Amber: caution. Never the same ANSI colour as rust.
    slot("warning", "warning", "Yellow"),
    // No IRONMAW token: structure (slate), used for sutures.
    slot("info", "structure", "DarkGray"),
    slot("syn_keyword", "accent", "Cyan"),
    slot("syn_string", "success", "Green"),
    slot("syn_var", "text", "Gray"),
    slot("syn_func", "structure-text", "LightBlue"),
    slot("syn_comment", "text-faint", "DarkGray"),
    slot("syn_number", "action-hover", "LightRed"),
    // The four IRONMAW roles the upstream engine had no slot for.
    slot("selected", "selected", "Blue"),
    slot("mark_bg", "mark-bg", "Black"),
    slot("border_strong", "border-strong", "DarkGray"),
    slot("action_hover", "action-hover", "LightRed"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenError {
    Json(String),
    WrongEdition { found: Option<String> },
    MissingColorGroup,
    MissingToken { token: &'static str },
    BadHex { token: &'static str, value: String },
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

/// Renders the generated module: `OXIDE_TRUECOLOR` and `OXIDE_ANSI16`.
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
        "/// IRONMAW Oxide, exact token colours (truecolor; quantised per cell for 256 colours).\n\
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
        "/// IRONMAW Oxide by role, for 16- and 8-colour terminals.\n\
         pub const OXIDE_ANSI16: ::raudbjorn_tui::theme::TerminalPalette = ::raudbjorn_tui::theme::TerminalPalette {{"
    );
    for (slot, _) in &resolved {
        let _ = writeln!(
            out,
            "    {}: ::ratatui::style::Color::{},",
            slot.field, slot.ansi16
        );
    }
    let _ = writeln!(out, "}};");
    Ok(out)
}
