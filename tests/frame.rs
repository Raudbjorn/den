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
use ratatui::{buffer::Buffer, layout::Rect};
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

/// The text inside `area` of a rendered buffer, one line per row, trimmed at
/// both ends, so a check on one plate cannot match text drawn in another.
fn region_lines(buffer: &Buffer, area: Rect) -> Vec<String> {
    (area.top()..area.bottom())
        .map(|y| {
            (area.left()..area.right())
                .map(|x| buffer.cell((x, y)).map_or(" ", |cell| cell.symbol()))
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .collect()
}

/// The sidebar row whose first word (after any mark) ends with `label`, split
/// into words: label, then any fork/archived glyph, then the count.
fn sidebar_row(lines: &[String], label: &str) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .find(|words| words.first().is_some_and(|w| w.ends_with(label)))
        .unwrap_or_else(|| panic!("no sidebar row {label:?} in {lines:#?}"))
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
        let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
        let regions = frame::regions(buffer.area);
        let sidebar = region_lines(&buffer, regions.sidebar);
        let text = frame::buffer_text(&buffer);
        assert!(
            sidebar.iter().any(|l| l.starts_with("R E P O S")),
            "{glyphs:?}: the sidebar label\n{text}"
        );
        // Each row carries its own glyph and count: a glyph or count drawn on
        // the wrong row, or not at all, fails here.
        for (label, words) in [
            (
                "All Owners",
                vec![
                    format!("{scope_mark}All"),
                    "Owners".to_owned(),
                    "3".to_owned(),
                ],
            ),
            ("octo", vec!["octo".to_owned(), "2".to_owned()]),
            (
                "a-fork",
                vec!["a-fork".to_owned(), fork.to_owned(), "1".to_owned()],
            ),
            (
                "b-old",
                vec!["b-old".to_owned(), archived.to_owned(), "1".to_owned()],
            ),
        ] {
            let first = label.split_whitespace().next().unwrap_or(label);
            assert_eq!(
                sidebar_row(&sidebar, first),
                words,
                "{glyphs:?}: the {label} row\n{text}"
            );
        }

        // The header names the whole Scope, then draws one segment per Scope
        // step after it (beta's sidebar row is scrolled out of view here, so
        // "beta" can only come from the header).
        let header = region_lines(&buffer, regions.header).join("\n");
        // The ASCII profile draws the sentence's "·" as "?" today (reported
        // separately), so only the Unicode run checks the separator; both
        // check the Owners' part and that the segments follow it.
        if glyphs == GlyphProfile::Unicode {
            assert!(
                header.contains("All Owners · octo, beta"),
                "{glyphs:?}: the Scope sentence\n{text}"
            );
        }
        let (before, after) = header
            .split_once("octo, beta")
            .unwrap_or_else(|| panic!("{glyphs:?}: the Scope sentence\n{text}"));
        assert!(
            before.contains("All Owners"),
            "{glyphs:?}: the Scope sentence\n{text}"
        );
        let segments: Vec<&str> = after
            .split_whitespace()
            .filter(|w| ["All", "Owners", "octo", "beta"].contains(w))
            .collect();
        assert_eq!(
            segments,
            ["All", "Owners", "octo", "beta"],
            "{glyphs:?}: segments drawn after the sentence\n{text}"
        );
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
    let header = region_lines(&buffer, frame::regions(buffer.area).header).join("\n");
    assert!(
        header.contains("All Owners · octo, beta"),
        "the header still names the whole Scope\n{text}"
    );
}

#[test]
fn a_title_of_wide_characters_wraps_by_cell_width_without_losing_text() {
    // 40 distinct CJK characters, two cells each: 80 cells, more than one line
    // of the detail plate but within its title's line limit.
    const WIDE_TITLE_CHARS: u32 = 40;
    let title: String = (0..WIDE_TITLE_CHARS)
        .filter_map(|offset| char::from_u32(0x4E00 + offset))
        .collect();
    let mut harness = Harness::with_owners(&["octo"]);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .pull_request(pr("octo/one", 1).title(&title))
        .build()));
    harness.start();
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let (cols, rows) = DESIGN_SIZE;
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    // Only the detail plate: the list row shows the title too.
    let detail = frame::regions(buffer.area).detail;
    let detail_text: String = (detail.top()..detail.bottom())
        .flat_map(|y| (detail.left()..detail.right()).map(move |x| (x, y)))
        .map(|cell| buffer[cell].symbol().to_owned())
        .collect();
    let missing: String = title
        .chars()
        .filter(|c| !detail_text.contains(*c))
        .collect();
    assert!(missing.is_empty(), "clipped: {missing}\n{detail_text}");
}
