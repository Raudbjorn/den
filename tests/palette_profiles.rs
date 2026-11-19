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
