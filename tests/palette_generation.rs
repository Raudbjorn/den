//! The Oxide palette is generated from the committed token file, and a changed
//! token changes the generated colour.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[allow(dead_code)]
#[path = "../build/palette_gen.rs"]
mod palette_gen;

use palette_gen::GenError;
use ratatui::style::Color;
use serde_json::Value;

const COMMITTED_TOKENS: &str = include_str!("../assets/tokens/oxide.tokens.json");
const COMPILED_PALETTE_SOURCE: &str = include_str!(concat!(env!("OUT_DIR"), "/oxide_palette.rs"));

fn token_rgb(tokens: &str, key: &str) -> Color {
    let doc: Value = serde_json::from_str(tokens).expect("valid JSON");
    let hex = doc["color"][key].as_str().expect("token is a string");
    let (r, g, b) = palette_gen::parse_hex(hex).expect("token is #RRGGBB");
    Color::Rgb(r, g, b)
}

fn with_token(key: &str, value: &str) -> String {
    let mut doc: Value = serde_json::from_str(COMMITTED_TOKENS).expect("valid JSON");
    doc["color"][key] = Value::String(value.to_owned());
    serde_json::to_string(&doc).expect("serialises")
}

#[test]
fn compiled_palette_is_exactly_the_committed_tokens() {
    let regenerated = palette_gen::generate(COMMITTED_TOKENS).expect("committed tokens generate");
    assert_eq!(COMPILED_PALETTE_SOURCE, regenerated);

    let oxide = den::palette::OXIDE_TRUECOLOR;
    for (field, token, actual) in [
        ("bg", "bg", oxide.bg),
        ("accent", "accent", oxide.accent),
        ("accent_rust", "action", oxide.accent_rust),
        ("warning", "warning", oxide.warning),
        ("selected", "selected", oxide.selected),
        ("mark_bg", "mark-bg", oxide.mark_bg),
        ("border_strong", "border-strong", oxide.border_strong),
        ("action_hover", "action-hover", oxide.action_hover),
    ] {
        assert_eq!(
            actual,
            token_rgb(COMMITTED_TOKENS, token),
            "{field} <- color.{token}"
        );
    }
}

#[test]
fn changing_a_token_changes_the_generated_colour() {
    let before = palette_gen::resolve(COMMITTED_TOKENS).expect("committed tokens resolve");
    let after =
        palette_gen::resolve(&with_token("selected", "#123456")).expect("edited tokens resolve");
    let colour = |resolved: &[(palette_gen::Slot, palette_gen::Rgb)], field: &str| {
        resolved
            .iter()
            .find(|(slot, _)| slot.field == field)
            .map(|(_, rgb)| *rgb)
    };
    assert_ne!(colour(&before, "selected"), colour(&after, "selected"));
    assert_eq!(colour(&after, "selected"), Some((0x12, 0x34, 0x56)));
    assert_eq!(
        colour(&before, "bg"),
        colour(&after, "bg"),
        "only the edited token moves"
    );

    let source = palette_gen::generate(&with_token("selected", "#123456")).expect("generates");
    assert!(source.contains("selected: ::ratatui::style::Color::Rgb(18, 52, 86)"));
}

#[test]
fn every_palette_slot_is_filled_from_a_token() {
    let resolved = palette_gen::resolve(COMMITTED_TOKENS).expect("committed tokens resolve");
    assert_eq!(resolved.len(), palette_gen::SLOTS.len());
}

#[test]
fn bad_token_files_are_errors_naming_the_token() {
    assert!(matches!(
        palette_gen::resolve(&with_token("selected", "teal")),
        Err(GenError::BadHex {
            token: "selected",
            ..
        })
    ));
    assert!(matches!(
        palette_gen::resolve(&with_token("selected", "#12345")),
        Err(GenError::BadHex {
            token: "selected",
            ..
        })
    ));

    let mut doc: Value = serde_json::from_str(COMMITTED_TOKENS).expect("valid JSON");
    doc["color"]
        .as_object_mut()
        .expect("color group")
        .remove("mark-bg");
    assert_eq!(
        palette_gen::resolve(&doc.to_string()),
        Err(GenError::MissingToken { token: "mark-bg" })
    );

    doc["$edition"] = Value::String("bone".to_owned());
    assert!(matches!(
        palette_gen::resolve(&doc.to_string()),
        Err(GenError::WrongEdition { .. })
    ));
    assert!(matches!(palette_gen::resolve("{"), Err(GenError::Json(_))));
}
