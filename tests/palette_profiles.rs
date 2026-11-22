//! The four IRONMAW roles the upstream engine had no slot for (`selected`,
//! `mark-bg`, `border-strong`, `action-hover`) must survive every colour profile.
//! The prototype substituted them after quantisation, so 256/16-colour lost them.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use crepuscularity_tui::TemplateContext;
use ratatui::{buffer::Buffer, style::Color, style::Modifier};
use raudbjorn_tui::{
    component::{TemplateFile, TemplateRef, TemplateSource},
    profile::ColorProfile,
};

const PROBE: TemplateFile = TemplateFile::new("role-probe");
const ROLES: TemplateRef = TemplateRef {
    file: PROBE,
    section: "Roles",
};

/// One cell per role: fill roles as backgrounds, line/ink roles as foregrounds,
/// plus the page background for comparison.
const ROLE_PROBE: &str = r#"
--- Roles
div w-full h-full flex-row
  span w-[1] bg-[#aa0017] text-[#aa0005] "s"
  span w-[1] bg-[#aa0018] text-[#aa0005] "m"
  span w-[1] bg-[#aa0000] text-[#aa0019] "b"
  span w-[1] bg-[#aa0000] text-[#aa001a] "a"
  span w-[1] bg-[#aa0000] text-[#aa0005] "g"
"#;
const PROBE_WIDTH: u16 = 5;
const PROBE_HEIGHT: u16 = 1;

struct Rendered {
    selected: Color,
    mark_bg: Color,
    border_strong: Color,
    action_hover: Color,
    page_bg: Color,
    cues: [Modifier; 4],
}

fn render_roles(color: ColorProfile) -> Rendered {
    let buffer: Buffer = support::render(
        vec![TemplateSource {
            file: PROBE,
            content: ROLE_PROBE,
        }],
        ROLES,
        &TemplateContext::default(),
        support::profile(color),
        PROBE_WIDTH,
        PROBE_HEIGHT,
    );
    assert_eq!(support::row_text(&buffer, 0), "smbag", "probe layout");
    let cell = |x: u16| &buffer[(x, 0)];
    Rendered {
        selected: cell(0).bg,
        mark_bg: cell(1).bg,
        border_strong: cell(2).fg,
        action_hover: cell(3).fg,
        page_bg: cell(4).bg,
        cues: [0, 1, 2, 3].map(|x| cell(x).modifier),
    }
}

fn is_sentinel(color: Color) -> bool {
    matches!(color, Color::Rgb(170, 0, _))
}

fn assert_distinct_real_colours(color: ColorProfile, accept: fn(Color) -> bool) {
    let r = render_roles(color);
    let roles = [
        ("selected", r.selected),
        ("mark-bg", r.mark_bg),
        ("border-strong", r.border_strong),
        ("action-hover", r.action_hover),
    ];
    for (name, role) in roles {
        assert_ne!(role, Color::Reset, "{color:?}: {name} reset");
        assert!(
            !is_sentinel(role),
            "{color:?}: {name} left as sentinel {role:?}"
        );
        assert!(
            accept(role),
            "{color:?}: {name} = {role:?} is not a {color:?} colour"
        );
    }
    for (i, (a_name, a)) in roles.iter().enumerate() {
        for (b_name, b) in &roles[i + 1..] {
            assert_ne!(a, b, "{color:?}: {a_name} and {b_name} merged into {a:?}");
        }
    }
    assert_ne!(
        r.selected, r.page_bg,
        "{color:?}: the selected fill is indistinguishable from the page"
    );
}

#[test]
fn extra_roles_stay_distinct_in_256_colours() {
    assert_distinct_real_colours(ColorProfile::Ansi256, |c| matches!(c, Color::Indexed(_)));
}

#[test]
fn extra_roles_stay_distinct_in_16_colours() {
    assert_distinct_real_colours(ColorProfile::Ansi16, |c| {
        !matches!(c, Color::Rgb(..) | Color::Indexed(_) | Color::Reset)
    });
}

#[test]
fn extra_roles_keep_their_token_colours_in_truecolor() {
    let r = render_roles(ColorProfile::TrueColor);
    let oxide = den::palette::OXIDE_TRUECOLOR;
    assert_eq!(r.selected, oxide.selected);
    assert_eq!(r.mark_bg, oxide.mark_bg);
    assert_eq!(r.border_strong, oxide.border_strong);
    assert_eq!(r.action_hover, oxide.action_hover);
}

#[test]
fn extra_roles_become_attribute_cues_in_mono_and_no_color() {
    for color in [ColorProfile::Mono, ColorProfile::NoColor] {
        let r = render_roles(color);
        for (role, name) in [r.selected, r.mark_bg, r.border_strong, r.action_hover]
            .into_iter()
            .zip(["selected", "mark-bg", "border-strong", "action-hover"])
        {
            assert_eq!(role, Color::Reset, "{color:?}: {name} still coloured");
        }
        for (i, cue) in r.cues.iter().enumerate() {
            assert!(!cue.is_empty(), "{color:?}: role {i} has no attribute cue");
            for other in &r.cues[i + 1..] {
                assert_ne!(cue, other, "{color:?}: two roles share the cue {cue:?}");
            }
        }
        assert!(
            r.cues[0].contains(Modifier::REVERSED),
            "selected reads as reversed"
        );
    }
}

#[test]
fn colour_profiles_add_no_attribute_cues() {
    for color in [
        ColorProfile::TrueColor,
        ColorProfile::Ansi256,
        ColorProfile::Ansi16,
    ] {
        let r = render_roles(color);
        assert!(
            r.cues.iter().all(Modifier::is_empty),
            "{color:?}: {:?}",
            r.cues
        );
    }
}

#[test]
fn rust_and_amber_stay_apart_in_16_colours() {
    // IRONMAW's "Amber Is Not Rust" rule; nearest-colour matching sends both to yellow.
    let ansi16 = den::palette::oxide_for(ColorProfile::Ansi16);
    assert_ne!(ansi16.accent_rust, ansi16.warning);
    assert_ne!(ansi16.accent_rust, ansi16.accent);
}

// #128: every palette role, one cell each, drawn as the foreground on the
// selected fill, so a test reads each role as the terminal gets it (after the
// sentinel pass and the profile's quantisation) and what sits on the fill.
const ALL_ROLES: TemplateRef = TemplateRef {
    file: PROBE,
    section: "AllRoles",
};
/// The engine's palette slots, in sentinel order (`TerminalPalette`).
const ROLE_NAMES: [&str; 27] = [
    "bg",
    "surface_1",
    "surface_2",
    "surface_3",
    "border",
    "text",
    "text_strong",
    "text_muted",
    "text_faint",
    "accent",
    "accent_2",
    "accent_rust",
    "mix_target",
    "success",
    "error",
    "warning",
    "syn_keyword",
    "syn_string",
    "syn_var",
    "syn_func",
    "syn_comment",
    "syn_number",
    "info",
    "selected",
    "mark_bg",
    "border_strong",
    "action_hover",
];
/// The roles den's templates draw on the selected fill (the list's current
/// row and the sidebar's cursor row).
const ON_SELECTED: [&str; 9] = [
    "text",
    "text_strong",
    "text_muted",
    "text_faint",
    "accent",
    "accent_2",
    "accent_rust",
    "warning",
    "info",
];
/// The fills that must stay apart: the page, the mark plate, the three
/// surfaces and the selected row.
const FILLS: [&str; 6] = [
    "bg",
    "mark_bg",
    "surface_1",
    "surface_2",
    "surface_3",
    "selected",
];

struct AllRoles {
    /// Each role's colour as drawn, by name.
    roles: Vec<(&'static str, Color)>,
    /// The selected fill behind them.
    selected_fill: Color,
}

impl AllRoles {
    fn of(&self, name: &str) -> Color {
        self.roles
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("no role {name}"))
    }
}

fn render_all_roles(color: ColorProfile) -> AllRoles {
    let probe: String = std::iter::once("--- AllRoles\ndiv w-full h-full flex-row\n".to_owned())
        .chain(
            (0..ROLE_NAMES.len())
                .map(|i| format!("  span w-[1] bg-[#aa0017] text-[#aa00{i:02x}] \"x\"\n")),
        )
        .collect();
    let buffer: Buffer = support::render(
        vec![TemplateSource {
            file: PROBE,
            content: Box::leak(probe.into_boxed_str()),
        }],
        ALL_ROLES,
        &TemplateContext::default(),
        support::profile(color),
        u16::try_from(ROLE_NAMES.len()).unwrap(),
        PROBE_HEIGHT,
    );
    AllRoles {
        roles: ROLE_NAMES
            .iter()
            .enumerate()
            .map(|(x, name)| (*name, buffer[(u16::try_from(x).unwrap(), 0)].fg))
            .collect(),
        selected_fill: buffer[(0, 0)].bg,
    }
}

/// The eight colours an 8-colour terminal has.
fn is_ansi8(color: Color) -> bool {
    matches!(
        color,
        Color::Black
            | Color::Red
            | Color::Green
            | Color::Yellow
            | Color::Blue
            | Color::Magenta
            | Color::Cyan
            | Color::Gray
    )
}

#[test]
fn in_8_colours_errors_and_structure_keep_their_roles() {
    let r = render_all_roles(ColorProfile::Ansi8);
    for (name, colour) in &r.roles {
        assert!(is_ansi8(*colour), "{name} = {colour:?} is not one of the 8");
    }
    // ADR-0001: rust is the one action; an error never draws in it.
    assert_ne!(r.of("error"), r.of("accent_rust"), "error drawn in rust");
    assert_ne!(r.of("syn_number"), r.of("accent_rust"));
    // Rust and amber stay apart, as in 16 colours.
    assert_ne!(r.of("accent_rust"), r.of("warning"));
    assert_ne!(r.of("accent_rust"), r.of("accent"));
    // Slate structure lines aren't body text.
    for line in ["border", "border_strong"] {
        assert_ne!(r.of(line), r.of("text"), "{line} looks like body text");
    }
    assert_ne!(r.selected_fill, r.of("bg"), "the selected fill is the page");
}

#[test]
fn in_256_colours_every_fill_and_error_stay_distinct() {
    let r = render_all_roles(ColorProfile::Ansi256);
    let fills: Vec<(&str, Color)> = FILLS.iter().map(|f| (*f, r.of(f))).collect();
    for (i, (a_name, a)) in fills.iter().enumerate() {
        assert!(matches!(a, Color::Indexed(_)), "{a_name} = {a:?}");
        for (b_name, b) in &fills[i + 1..] {
            assert_ne!(a, b, "{a_name} and {b_name} merged into {a:?}");
        }
    }
    assert_ne!(r.of("error"), r.of("action_hover"));
    assert_ne!(r.of("error"), r.of("accent_rust"));
}

#[test]
fn nothing_drawn_on_the_selected_fill_disappears_into_it() {
    for color in [
        ColorProfile::TrueColor,
        ColorProfile::Ansi256,
        ColorProfile::Ansi16,
        ColorProfile::Ansi8,
    ] {
        let r = render_all_roles(color);
        for name in ON_SELECTED {
            assert_ne!(
                r.of(name),
                r.selected_fill,
                "{color:?}: {name} is invisible on the selected fill"
            );
        }
    }
}
