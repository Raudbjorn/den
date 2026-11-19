//! Smoke checks for the Tonal-plates frame. How it looks is verified by hand
//! (spec, Testing Decisions); these only catch a frame that fails to render or
//! loses its plates, before and after a survey.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{App, ViewModel},
    palette::OXIDE_TRUECOLOR,
    ui::{Theme, frame},
};
use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};
use support::{
    build::{ME, ObservationBuilder, pr},
    harness::{Harness, config},
};

const DESIGN_SIZE: (u16, u16) = (160, 45);
const MINIMUM_SIZE: (u16, u16) = (80, 24);

/// The view before any survey: the empty frame.
fn unsurveyed() -> ViewModel {
    App::new(Ok(config(&["octo"]))).view()
}

/// A view with rows, including text the template language might misread.
fn surveyed() -> ViewModel {
    let mut harness = Harness::with_owners(&["octo"]);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .pull_request(
            pr("octo/one", 1)
                .draft()
                .author(ME)
                .title("Braces {title} and \"quotes\""),
        )
        .pull_request(pr("octo/two", 2))
        .build()));
    harness.start().clone()
}

#[test]
fn unsurveyed_frame_renders_in_every_profile_at_design_and_minimum_size() {
    let colors = [
        ColorProfile::TrueColor,
        ColorProfile::Ansi256,
        ColorProfile::Ansi16,
        ColorProfile::Ansi8,
        ColorProfile::Mono,
        ColorProfile::NoColor,
    ];
    for color in colors {
        for glyphs in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
            let theme = Theme::new(TerminalProfile::new(color, glyphs)).expect("theme");
            for (cols, rows) in [DESIGN_SIZE, MINIMUM_SIZE] {
                let buffer = frame::render(&theme, &unsurveyed(), cols, rows).expect("renders");
                let text = frame::buffer_text(&buffer);
                for expected in ["DEN", "No survey yet.", "D E T A I L", "quit"] {
                    assert!(
                        text.contains(expected),
                        "{color:?}/{glyphs:?} {cols}x{rows}: missing {expected:?}\n{text}"
                    );
                }
            }
        }
    }
}

#[test]
fn plates_are_told_apart_by_tone_in_truecolor() {
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::TrueColor,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let (cols, rows) = DESIGN_SIZE;
    let buffer = frame::render(&theme, &unsurveyed(), cols, rows).expect("renders");
    let regions = frame::regions(buffer.area);
    let bg_at = |x: u16, y: u16| buffer[(x, y)].bg;

    assert_eq!(
        bg_at(regions.header.x, regions.header.y),
        OXIDE_TRUECOLOR.mark_bg
    );
    assert_eq!(bg_at(regions.list.x, regions.list.y), OXIDE_TRUECOLOR.bg);
    assert_eq!(
        bg_at(regions.sidebar.x, regions.sidebar.y),
        OXIDE_TRUECOLOR.surface_1
    );
    assert_eq!(
        buffer[(regions.sidebar_suture.x, regions.sidebar_suture.y)].symbol(),
        "┊"
    );
    let inside_detail = (regions.detail.x + 1, regions.detail.y + 1);
    assert_eq!(
        bg_at(inside_detail.0, inside_detail.1),
        OXIDE_TRUECOLOR.surface_2
    );
    assert_eq!(buffer[(regions.suture.x, regions.suture.y)].symbol(), "┊");
    assert_eq!(buffer[(regions.detail.x, regions.detail.y)].symbol(), "◢");
}

#[test]
fn surveyed_frame_shows_rows_and_the_selected_items_detail_at_every_size() {
    let view = surveyed();
    for glyphs in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
        let theme = Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
        // At 80 columns the fixed detail plate squeezes the list; the narrow
        // layout (detail as a sheet) is the responsive-layout ticket's (#10).
        let (cols, rows) = DESIGN_SIZE;
        {
            let text =
                frame::buffer_text(&frame::render(&theme, &view, cols, rows).expect("renders"));
            for expected in [
                "Finish draft",
                "octo/one #1",
                "octo/two #2",
                "no step yet",
                "NEXT STEP",
            ] {
                assert!(
                    text.contains(expected),
                    "{glyphs:?} {cols}x{rows}: missing {expected:?}\n{text}"
                );
            }
            assert!(
                text.contains("Braces {title} and"),
                "title text is literal\n{text}"
            );
        }
    }
}

#[test]
fn a_long_list_keeps_the_selection_in_view() {
    let mut builder = ObservationBuilder::new().owner("octo");
    for number in 1..=40 {
        builder = builder.pull_request(
            pr("octo/many", number).updated(&format!("2026-11-{:02}T00:00:00Z", 1 + number % 18)),
        );
    }
    let mut harness = Harness::with_owners(&["octo"]);
    harness.answer(Ok(builder.build()));
    harness.start();
    for _ in 0..39 {
        harness.key(den::app::Key::Down);
    }
    let selected = harness.view.list.selected.expect("a selection");
    let reference = harness.view.list.rows[selected].reference.clone();
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Ascii,
    ))
    .expect("theme");
    let (cols, rows) = DESIGN_SIZE;
    let text =
        frame::buffer_text(&frame::render(&theme, &harness.view, cols, rows).expect("renders"));
    assert!(
        text.contains(&format!("octo/many {reference}")),
        "selected {reference} not drawn\n{text}"
    );
    assert!(text.contains("40 of 40"), "{text}");
}

/// Many Repos under two Owners, one a fork and one archived, with Work Items.
fn many_repos() -> Harness {
    let mut builder = ObservationBuilder::new()
        .owner("octo")
        .owner("beta")
        .fork("octo/a-fork", Some("up/a-fork"))
        .archived_repo("octo/b-old");
    for n in 0..60 {
        builder = builder.repo(&format!("octo/repo-{n:02}"));
    }
    builder = builder
        .repo("beta/zeta")
        .pull_request(pr("octo/a-fork", 1).draft().author(ME))
        .pull_request(pr("octo/b-old", 2).draft().author("alice"))
        .pull_request(pr("beta/zeta", 3).draft().author(ME));
    let mut harness = Harness::with_owners(&["octo", "beta"]);
    harness.answer(Ok(builder.build()));
    harness.start();
    harness
}

#[test]
fn the_sidebar_shows_repos_with_counts_and_glyphs_and_the_header_its_segments() {
    let harness = many_repos();
    for (glyphs, fork, archived, scope_mark) in [
        (GlyphProfile::Unicode, "⑂", "▣", "●"),
        (GlyphProfile::Ascii, "Y", "#", "*"),
    ] {
        let theme = Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
        let (cols, rows) = DESIGN_SIZE;
        let text =
            frame::buffer_text(&frame::render(&theme, &harness.view, cols, rows).expect("renders"));
        for expected in [
            "R E P O S",
            &format!("{scope_mark}All Owners"),
            "  a-fork",
            fork,
            archived,
            "beta",
        ] {
            assert!(
                text.contains(expected),
                "{glyphs:?}: missing {expected:?}\n{text}"
            );
        }
        assert!(text.contains("octo, beta"), "the Scope sentence\n{text}");
        assert!(text.contains(" octo "), "segments drawn\n{text}");
    }
}

#[test]
fn a_long_sidebar_keeps_its_cursor_in_view() {
    let mut harness = many_repos();
    harness.key(den::app::Key::Tab);
    // Down to beta/zeta, the last row.
    for _ in 0..80 {
        harness.key(den::app::Key::Down);
    }
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Ascii,
    ))
    .expect("theme");
    let (cols, rows) = DESIGN_SIZE;
    let text =
        frame::buffer_text(&frame::render(&theme, &harness.view, cols, rows).expect("renders"));
    assert!(text.contains(">   zeta"), "cursor row drawn\n{text}");
    assert!(!text.contains("repo-00"), "scrolled past the top\n{text}");
}

#[test]
fn the_sidebar_gives_way_to_the_list_at_the_minimum_size() {
    let harness = many_repos();
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let (cols, rows) = MINIMUM_SIZE;
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    assert!(frame::regions(buffer.area).sidebar.is_empty());
    let text = frame::buffer_text(&buffer);
    assert!(
        text.contains("Owners"),
        "the header still names the Scope\n{text}"
    );
}
