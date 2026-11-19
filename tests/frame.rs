//! Smoke checks for the Tonal-plates frame. How it looks is verified by hand
//! (spec, Testing Decisions); these only catch a frame that fails to render or
//! loses its plates, before and after a survey.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{App, Event, Key, ViewModel, view_model::EmptyState},
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

/// The view before any survey, laid out for `cols` x `rows`: the empty frame.
fn unsurveyed((cols, rows): (u16, u16)) -> ViewModel {
    let mut app = App::new(Ok(config(&["octo"])));
    app.handle(Event::Resized { cols, rows }).0
}

/// A view with rows, including text the template language might misread.
fn surveyed() -> ViewModel {
    surveyed_harness(DESIGN_SIZE).view
}

fn surveyed_harness((cols, rows): (u16, u16)) -> Harness {
    let mut harness = Harness::with_owners(&["octo"]);
    harness.send(Event::Resized { cols, rows });
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
    harness.start();
    harness
}

fn truecolor() -> Theme {
    Theme::new(TerminalProfile::new(
        ColorProfile::TrueColor,
        GlyphProfile::Unicode,
    ))
    .expect("theme")
}

/// Cells drawn as half-block bear pixels (the off-screen bear) inside `area`.
fn bear_cells(buffer: &Buffer, area: Rect) -> usize {
    let area = area.intersection(buffer.area);
    (area.top()..area.bottom())
        .flat_map(|y| (area.left()..area.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| matches!(buffer[(x, y)].symbol(), "▀" | "▄" | "█"))
        .count()
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
                let view = unsurveyed((cols, rows));
                let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
                let text = frame::buffer_text(&buffer);
                for expected in ["DEN", "No survey yet.", "quit"] {
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
    let view = unsurveyed(DESIGN_SIZE);
    let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
    let regions = frame::regions(buffer.area, &view.layout);
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
    harness.send(Event::Resized {
        cols: DESIGN_SIZE.0,
        rows: DESIGN_SIZE.1,
    });
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

#[test]
fn the_detail_plate_appears_at_the_wide_threshold_and_not_below_it() {
    use den::layout::WIDE_LAYOUT_MIN_COLS;
    let theme = truecolor();
    let rows = MINIMUM_SIZE.1;
    for (cols, pane) in [
        (WIDE_LAYOUT_MIN_COLS - 1, false),
        (WIDE_LAYOUT_MIN_COLS, true),
    ] {
        let view = surveyed_harness((cols, rows)).view;
        let text = frame::buffer_text(&frame::render(&theme, &view, cols, rows).expect("renders"));
        assert_eq!(text.contains("D E T A I L"), pane, "{cols} columns\n{text}");
        // The list keeps the rows either way.
        assert!(text.contains("octo/one #1"), "{cols} columns\n{text}");
    }
}

#[test]
fn at_80_by_24_enter_draws_the_detail_as_a_sheet_over_the_list() {
    let mut harness = surveyed_harness(MINIMUM_SIZE);
    harness.key(Key::Enter);
    let (cols, rows) = MINIMUM_SIZE;
    for glyphs in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
        let theme = Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
        let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
        let text = frame::buffer_text(&buffer);
        let sheet = frame::overlay_area(&harness.view, buffer.area).expect("a sheet");
        assert!(sheet.width > 0 && sheet.height > 0, "{sheet:?}");
        for expected in ["D E T A I L", "NEXT STEP", "WAITING ON", "esc"] {
            assert!(
                text.contains(expected),
                "{glyphs:?}: missing {expected:?}\n{text}"
            );
        }
    }
}

#[test]
fn the_header_bear_is_drawn_in_true_colour_and_left_out_in_monochrome() {
    let view = unsurveyed(DESIGN_SIZE);
    let (cols, rows) = DESIGN_SIZE;
    let header = Rect::new(0, 0, cols, frame::HEADER_PLATE_ROWS);

    let buffer = frame::render(&truecolor(), &view, cols, rows).expect("renders");
    assert!(
        bear_cells(&buffer, header) > 0,
        "{}",
        frame::buffer_text(&buffer)
    );

    let mono = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let buffer = frame::render(&mono, &view, cols, rows).expect("renders");
    assert_eq!(
        bear_cells(&buffer, header),
        0,
        "{}",
        frame::buffer_text(&buffer)
    );
}

#[test]
fn the_splash_draws_the_large_bear() {
    let mut harness = Harness::with_owners(&["octo"]);
    harness.send(Event::Resized {
        cols: DESIGN_SIZE.0,
        rows: DESIGN_SIZE.1,
    });
    harness.send(Event::Started {
        now: support::build::now(),
    });
    assert!(harness.view.splash);
    let (cols, rows) = DESIGN_SIZE;
    let buffer = frame::render(&truecolor(), &harness.view, cols, rows).expect("renders");
    let text = frame::buffer_text(&buffer);
    assert!(text.contains("DEN") && text.contains("Surveying"), "{text}");
    let header = Rect::new(0, 0, cols, frame::HEADER_PLATE_ROWS);
    assert!(
        bear_cells(&buffer, buffer.area) > bear_cells(&buffer, header) * 4,
        "{text}"
    );
}

/// A sheet open over the empty state: the empty-state bear would sit under it.
/// (Today only by hand-building the view: the detail sheet needs a selection,
/// but a refresh can empty the list under it, and the launch sheet comes next.)
#[test]
fn an_overlay_never_has_bear_pixels_over_it() {
    let (cols, rows) = MINIMUM_SIZE;
    let mut harness = surveyed_harness(MINIMUM_SIZE);
    harness.key(Key::Enter);
    let mut view = harness.view.clone();
    view.list.rows.clear();
    view.list.selected = None;
    view.list.empty = Some(EmptyState {
        title: "Nothing needs you.".to_owned(),
        detail: "Every source was read completely.".to_owned(),
    });
    let theme = truecolor();
    let regions = frame::regions(Rect::new(0, 0, cols, rows), &view.layout);

    // Without the sheet, the large bear is in the list plate.
    let mut closed = view.clone();
    closed.overlay = None;
    let buffer = frame::render(&theme, &closed, cols, rows).expect("renders");
    assert!(
        bear_cells(&buffer, regions.list) > 0,
        "{}",
        frame::buffer_text(&buffer)
    );

    // With it, that bear is left out entirely, not just where the sheet is.
    let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
    let sheet = frame::overlay_area(&view, buffer.area).expect("a sheet");
    assert_eq!(
        bear_cells(&buffer, sheet),
        0,
        "{}",
        frame::buffer_text(&buffer)
    );
    assert_eq!(
        bear_cells(&buffer, regions.list),
        0,
        "{}",
        frame::buffer_text(&buffer)
    );
}

#[test]
fn a_terminal_below_80_by_24_says_so_in_the_footer() {
    let (cols, rows) = (60, 20);
    let view = unsurveyed((cols, rows));
    let mono = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Ascii,
    ))
    .expect("theme");
    let text = frame::buffer_text(&frame::render(&mono, &view, cols, rows).expect("renders"));
    assert!(text.contains("den needs at least 80x24"), "{text}");
}

/// Many Repos under two Owners, one a fork and one archived, with Work Items,
/// at `size`.
fn many_repos((cols, rows): (u16, u16)) -> Harness {
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
    harness.send(Event::Resized { cols, rows });
    harness.answer(Ok(builder.build()));
    harness.start();
    harness
}

#[test]
fn the_sidebar_shows_repos_with_counts_and_glyphs_and_the_header_its_segments() {
    let harness = many_repos(DESIGN_SIZE);
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
    let mut harness = many_repos(DESIGN_SIZE);
    harness.key(Key::Tab);
    // Down to beta/zeta, the last row.
    for _ in 0..80 {
        harness.key(Key::Down);
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

/// #10's narrow layout keeps the sidebar (the rail) at 80x24; only the detail
/// pane gives way (to a sheet).
#[test]
fn the_sidebar_stays_beside_the_list_at_the_minimum_size() {
    let harness = many_repos(MINIMUM_SIZE);
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let (cols, rows) = MINIMUM_SIZE;
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    let regions = frame::regions(buffer.area, &harness.view.layout);
    assert!(!regions.sidebar.is_empty());
    assert!(regions.detail.is_empty());
    let text = frame::buffer_text(&buffer);
    assert!(text.contains("R E P O S"), "the sidebar is drawn\n{text}");
    assert!(
        text.contains("Owners"),
        "the header still names the Scope\n{text}"
    );
}
