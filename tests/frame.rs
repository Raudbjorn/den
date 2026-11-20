//! Smoke checks for the Tonal-plates frame. How it looks is verified by hand
//! (spec, Testing Decisions); these only catch a frame that fails to render or
//! loses its plates, before and after a survey.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{
        App, Event, Key, ViewModel,
        view_model::{ELLIPSIS, ENTER_KEY, EmptyState, SEPARATOR, SidebarEntry},
    },
    palette::OXIDE_TRUECOLOR,
    ui::{Theme, frame},
};
use ratatui::{buffer::Buffer, layout::Rect};
use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};
use support::{
    build::{ME, ObservationBuilder, pr},
    harness::{Harness, config},
};
use unicode_width::UnicodeWidthStr;

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
/// into words: label, then any fork/archived glyph, any incomplete mark, then
/// the count.
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
    let harness = many_repos(DESIGN_SIZE);
    // Every row is marked incomplete: the builder leaves Branches and Runs
    // unfetched, so no Scope here was read completely.
    for (glyphs, fork, archived, scope_mark, incomplete, separator) in [
        (GlyphProfile::Unicode, "⑂", "▣", "●", "⚠", "·"),
        (GlyphProfile::Ascii, "Y", "#", "*", "!", "-"),
    ] {
        let theme = Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
        let (cols, rows) = DESIGN_SIZE;
        let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
        let regions = frame::regions(buffer.area, &harness.view.layout);
        let sidebar = region_lines(&buffer, regions.sidebar);
        let text = frame::buffer_text(&buffer);
        assert!(
            sidebar.iter().any(|l| l.starts_with("R E P O S")),
            "{glyphs:?}: the sidebar label\n{text}"
        );
        // Each row carries its own glyphs and count: a glyph or count drawn on
        // the wrong row, or not at all, fails here.
        for (label, words) in [
            (
                "All Owners",
                vec![
                    format!("{scope_mark}All"),
                    "Owners".to_owned(),
                    incomplete.to_owned(),
                    "3".to_owned(),
                ],
            ),
            (
                "octo",
                vec!["octo".to_owned(), incomplete.to_owned(), "2".to_owned()],
            ),
            (
                "a-fork",
                vec![
                    "a-fork".to_owned(),
                    fork.to_owned(),
                    incomplete.to_owned(),
                    "1".to_owned(),
                ],
            ),
            (
                "b-old",
                vec![
                    "b-old".to_owned(),
                    archived.to_owned(),
                    incomplete.to_owned(),
                    "1".to_owned(),
                ],
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
        // The sentence's separator has an ASCII form of its own (not the
        // engine's "?" for a character it cannot draw).
        assert!(
            header.contains(&format!("All Owners {separator} octo, beta")),
            "{glyphs:?}: the Scope sentence\n{text}"
        );
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

/// The ViewModel's separator and ellipsis each have an ASCII form, so an ASCII
/// header never shows the engine's "?" for them. (The status is set on the view:
/// a refresh in flight is otherwise only visible between two settles.)
#[test]
fn an_ascii_header_draws_separators_and_ellipses_in_ascii() {
    let mut view = surveyed_harness(DESIGN_SIZE).view;
    view.header.status = format!("observed 12:00 UTC {SEPARATOR} refreshing{ELLIPSIS}");
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Ascii,
    ))
    .expect("theme");
    let (cols, rows) = DESIGN_SIZE;
    let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
    let header = region_lines(&buffer, frame::regions(buffer.area, &view.layout).header).join("\n");
    assert!(header.contains("All Owners - octo"), "{header}");
    assert!(
        header.contains("observed 12:00 UTC - refreshing..."),
        "{header}"
    );
    assert!(!header.contains('?'), "{header}");
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
    let header = region_lines(&buffer, regions.header).join("\n");
    assert!(
        header.contains("All Owners · octo, beta"),
        "the header still names the whole Scope\n{text}"
    );
}

/// A terminal far taller than any design size, with many Work Items and Repos.
const TALL_SIZE: (u16, u16) = (160, 400);
/// A tall frame must draw well within this, even in a debug build. Before the
/// list, sidebar and sutures stopped handing the template engine one element
/// per row, a frame like this took over a second in a release build (and a
/// 160x2200 `--dump` ran for minutes).
const TALL_REPOS: u64 = 120;
const TALL_WORK_ITEMS: u64 = 240;
const TALL_FRAME_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

#[test]
fn a_tall_terminal_draws_its_visible_rows_in_bounded_time() {
    let (cols, rows) = TALL_SIZE;
    let mut builder = ObservationBuilder::new().owner("octo");
    for n in 0..TALL_REPOS {
        builder = builder.repo(&format!("octo/repo-{n:03}"));
    }
    // More Work Items than the 198 two-line rows a 400-row list plate holds.
    for n in 0..TALL_WORK_ITEMS {
        builder =
            builder.pull_request(pr(&format!("octo/repo-{:03}", n % TALL_REPOS), n + 1).draft());
    }
    let mut harness = Harness::with_owners(&["octo"]);
    harness.send(Event::Resized { cols, rows });
    harness.answer(Ok(builder.build()));
    harness.start();
    // The last row selected, so the list window must scroll (set on the view:
    // 600 key presses would only slow the test down).
    let mut view = harness.view.clone();
    view.list.selected = Some(view.list.rows.len() - 1);
    let theme = truecolor();
    // The first frame at a size, when no layout is cached yet: the worst case.
    let started = std::time::Instant::now();
    let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
    let took = started.elapsed();
    assert!(took < TALL_FRAME_BUDGET, "a 160x400 frame took {took:?}");
    // And every frame after it: the layouts it needs must all stay cached.
    let started = std::time::Instant::now();
    frame::render(&theme, &view, cols, rows).expect("renders");
    let took = started.elapsed();
    assert!(
        took < TALL_FRAME_BUDGET,
        "the next 160x400 frame took {took:?}"
    );

    let text = frame::buffer_text(&buffer);
    let selected = &view.list.rows[view.list.selected.expect("selected")];
    assert!(
        text.contains(&format!("{} {}", selected.repo, selected.reference)),
        "the selected row is in view"
    );
    let first = &view.list.rows[0];
    assert!(
        !text.contains(&format!("{} {} ", first.repo, first.reference)),
        "the list scrolled past its first row"
    );
    assert!(
        text.contains(&format!("{TALL_WORK_ITEMS} of {TALL_WORK_ITEMS}")),
        "the position"
    );
    assert!(text.contains("repo-000"), "the sidebar's first Repo");
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
    // Wide enough for the detail plate beside the list.
    let (cols, rows) = DESIGN_SIZE;
    harness.send(Event::Resized { cols, rows });
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
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    // Only the detail plate: the list row shows the title too.
    let detail = frame::regions(buffer.area, &harness.view.layout).detail;
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

/// At the minimum size every key hint is drawn whole, in either focus and
/// either glyph profile: each hint takes its own width, and the Enter key has an
/// ASCII form. (Fixed-width hints overflowed 80 columns once the narrow
/// layout's Enter hint, or the sidebar's three, joined the rest.)
#[test]
fn every_key_hint_fits_the_footer_at_80_by_24() {
    let mut harness = surveyed_harness(MINIMUM_SIZE);
    let (cols, rows) = MINIMUM_SIZE;
    for focus in ["list", "sidebar"] {
        if focus == "sidebar" {
            harness.key(Key::Tab);
        }
        assert!(
            harness.view.footer.hints.len() >= 6,
            "{focus}: {:?}",
            harness.view.footer
        );
        for (glyphs, enter) in [(GlyphProfile::Unicode, "⏎"), (GlyphProfile::Ascii, "RET")] {
            let theme =
                Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
            let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
            let footer = frame::buffer_text(&buffer)
                .lines()
                .last()
                .unwrap_or_default()
                .to_owned();
            for hint in &harness.view.footer.hints {
                let key = if hint.key == ENTER_KEY.to_string() {
                    enter.to_owned()
                } else if glyphs == GlyphProfile::Ascii {
                    // The engine's own ASCII forms for arrows.
                    hint.key
                        .replace('↑', "^")
                        .replace('↓', "v")
                        .replace('←', "<")
                        .replace('→', ">")
                } else {
                    hint.key.clone()
                };
                let drawn = format!("{key} {}", hint.label);
                assert!(
                    footer.contains(&drawn),
                    "{focus}/{glyphs:?}: {drawn:?} in {footer:?}"
                );
            }
            assert!(!footer.contains('?'), "{focus}/{glyphs:?}: {footer:?}");
        }
    }
}

#[test]
fn every_detail_page_is_reachable_in_the_pane_and_narrow_sheet() {
    use den::domain::{Incompleteness, SourceKind, SourceState};
    for (cols, rows) in [(110, 24), (80, 24)] {
        for glyphs in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
            let theme = Theme::new(TerminalProfile::new(ColorProfile::NoColor, glyphs)).unwrap();
            let mut rich = pr("octo/one", 1).author(ME).threads_truncated();
            for n in 0..25 {
                rich = rich.failing_required(&format!("CHECK-END-{n}"));
            }
            let mut harness = Harness::with_owners(&["octo"]);
            harness.answer(Ok(ObservationBuilder::new()
                .fully_read_owner("octo")
                .pull_request(rich)
                .pull_request(pr("octo/two", 2))
                .repo_coverage(
                    "octo/one",
                    SourceKind::PullRequestDetails,
                    SourceState::Partial(Incompleteness::FollowUpTruncated {
                        what: "threads".to_owned(),
                    }),
                )
                .build()));
            harness.send(Event::Resized { cols, rows });
            harness.start();
            if cols < 110 {
                harness.key(Key::Enter);
            }
            let draw = |view: &ViewModel| {
                frame::buffer_text(&frame::render(&theme, view, cols, rows).unwrap())
            };
            let first = draw(&harness.view);
            assert!(first.contains("PgUp/PgDn"));
            let selected = harness.view.list.selected;
            let mut all = first.clone();
            let mut last = first.clone();
            let mut wrapped = false;
            for _ in 0..128 {
                harness.key(Key::PageDown);
                assert_eq!(harness.view.list.selected, selected);
                let page = draw(&harness.view);
                if page == first {
                    wrapped = true;
                    break;
                }
                all.push_str(&page);
                last = page;
            }
            assert!(wrapped, "pages cycle through the finite detail content");
            // The decision engine lists four check names and summarizes the rest.
            for text in [
                "CHECK-END-3",
                "21 more",
                "INCOMPLETE DATA",
                "UNKNOWN",
                "OBSERVED",
            ] {
                assert!(all.contains(text), "{cols}x{rows}: missing {text}\n{all}");
            }
            harness.key(Key::PageUp);
            assert_eq!(draw(&harness.view), last, "Page Up wraps to the last page");
            harness.key(Key::Down);
            assert_eq!(
                harness.view.detail.page, 0,
                "a different item starts on page one"
            );
            harness.key(Key::PageDown);
            harness.send(Event::Resized {
                cols,
                rows: rows + 1,
            });
            assert_eq!(harness.view.detail.page, 0, "resize resets wrapping");
        }
    }
}

/// A label of `cells` cells broken by hyphens, the way Repo names and workflow
/// paths are: left to the template, such text wraps at a hyphen and loses its
/// end without a sign.
fn hyphenated(cells: usize) -> String {
    "ab-".repeat(cells).chars().take(cells).collect()
}

/// The profile's ellipsis: the ViewModel's, in that profile's form.
fn ellipsis(glyphs: GlyphProfile) -> &'static str {
    match glyphs {
        GlyphProfile::Unicode => "…",
        GlyphProfile::Ascii => "...",
    }
}

/// A 4-digit count, as an Owner with many Work Items has ("1101┊" touched the
/// rail at 80 columns).
const WIDE_COUNT: usize = 1101;

/// The sidebar rows the sweep puts the cursor on, in `many_repos`: All Owners,
/// the Owner `octo` (both headings), and the Repo `octo/a-fork`. The template
/// draws the cursor's row in a branch of its own, so each kind is swept there.
const SIDEBAR_CURSORS: [usize; 3] = [0, 1, 2];

/// At either size and in either glyph profile, a count as wide as its column
/// keeps a blank cell before the sidebar's rail, and a label too long for what
/// is left is cut with the profile's ellipsis instead of pushing into the
/// count or losing its end unmarked. Swept over every label length up to the
/// plate's width, so whichever length exactly fills the label column is hit,
/// with the cursor on a heading and on a Repo.
#[test]
fn a_wide_sidebar_count_keeps_off_the_rail_and_long_labels_end_in_an_ellipsis() {
    let mut problems = Vec::new();
    for size in [MINIMUM_SIZE, DESIGN_SIZE] {
        let harness = many_repos(size);
        let (cols, rows) = size;
        for (glyphs, cursor) in [GlyphProfile::Unicode, GlyphProfile::Ascii]
            .into_iter()
            .flat_map(|glyphs| SIDEBAR_CURSORS.map(|cursor| (glyphs, cursor)))
        {
            let theme =
                Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
            let mark = ellipsis(glyphs);
            let width = frame::regions(Rect::new(0, 0, cols, rows), &harness.view.layout)
                .sidebar
                .width;
            for cells in 1..=usize::from(width) {
                let mut view = harness.view.clone();
                view.sidebar.focused = true;
                view.sidebar.cursor = Some(cursor);
                let label = hyphenated(cells);
                for (i, row) in view.sidebar.rows.iter_mut().enumerate() {
                    row.label.clone_from(&label);
                    row.count = WIDE_COUNT;
                    // Every other Repo targets its Upstream: the widest row.
                    // Only a Repo has an Upstream in a real ViewModel.
                    if i % 2 == 1 && row.entry == SidebarEntry::Repo && row.upstream.is_none() {
                        row.upstream = Some("up/stream".to_owned());
                    }
                }
                let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
                let sidebar = frame::regions(buffer.area, &view.layout).sidebar;
                let context = format!(
                    "{glyphs:?} {cols}x{rows}, cursor on row {cursor}, label of {cells} cells"
                );
                let first_row = sidebar.y + 3;
                for y in first_row..sidebar.bottom() {
                    let line: String = (sidebar.left()..sidebar.right())
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    if line.trim().is_empty() {
                        continue;
                    }
                    if !line.ends_with(&format!(" {WIDE_COUNT} ")) {
                        problems.push(format!(
                            "{context}: not a blank, the count, a blank before the rail: {line:?}"
                        ));
                    }
                    if !(line.contains(&label) || line.contains(mark)) {
                        problems.push(format!(
                            "{context}: the label neither whole nor cut with {mark:?}: {line:?}"
                        ));
                    }
                }
            }
        }
    }
    assert_no_problems(&problems);
}

/// Fails listing every problem found (a sweep reports all of its misses, in
/// every profile and size, not only the first).
fn assert_no_problems(problems: &[String]) {
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// A count wider than its column is drawn as a true bound, not cut to digits.
#[test]
fn a_five_digit_sidebar_count_is_drawn_as_a_bound_within_its_column() {
    let mut harness = many_repos(MINIMUM_SIZE);
    harness.view.sidebar.rows[0].count = 12_345;
    let (cols, rows) = MINIMUM_SIZE;
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Ascii,
    ))
    .expect("theme");
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    let sidebar = region_lines(
        &buffer,
        frame::regions(buffer.area, &harness.view.layout).sidebar,
    );
    let all = sidebar_row(&sidebar, "All");
    assert_eq!(all.last().map(String::as_str), Some("10k+"), "{sidebar:#?}");
}

/// The age a Run row had when its workflow path ran into it ("release.yml146d").
const OLD_AGE: &str = "146d";

/// At either size and in either glyph profile, a Work Item row's Repo and
/// reference keep a blank cell before the age, and its title one before Waiting
/// On; either too long for its column is cut with the profile's ellipsis
/// rather than running into the next column or losing its end unmarked. Swept
/// over every length up to the plate's width, selected row and not.
#[test]
fn a_long_workflow_path_and_title_are_cut_with_an_ellipsis_clear_of_the_next_column() {
    let mut problems = Vec::new();
    for size in [MINIMUM_SIZE, DESIGN_SIZE] {
        let harness = surveyed_harness(size);
        let (cols, rows) = size;
        for glyphs in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
            let theme =
                Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
            let mark = ellipsis(glyphs);
            let width = frame::regions(Rect::new(0, 0, cols, rows), &harness.view.layout)
                .list
                .width;
            for cells in 1..=usize::from(width) {
                let mut view = harness.view.clone();
                let path = format!(".github/workflows/{}", hyphenated(cells));
                let title = hyphenated(cells);
                for row in &mut view.list.rows {
                    row.reference.clone_from(&path);
                    row.title.clone_from(&title);
                    row.age = OLD_AGE.to_owned();
                }
                let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
                let list = frame::regions(buffer.area, &view.layout).list;
                let lines = region_lines(&buffer, list);
                let text = lines.join("\n");
                let context = format!("{glyphs:?} {cols}x{rows}, {cells} cells");
                for row in &view.list.rows {
                    let place = format!("{} {path}", row.repo);
                    let first = lines
                        .iter()
                        .position(|l| l.contains(&row.repo))
                        .unwrap_or_else(|| panic!("{context}: no row for {}\n{text}", row.repo));
                    let (top, bottom) = (&lines[first], &lines[first + 1]);
                    if !top.ends_with(&format!(" {OLD_AGE}")) {
                        problems.push(format!("{context}: no blank before the age: {top:?}"));
                    }
                    if !(top.contains(&place) || top.contains(mark)) {
                        problems.push(format!(
                            "{context}: Repo and path neither whole nor cut with {mark:?}: {top:?}"
                        ));
                    }
                    if !bottom.ends_with(&format!(" {}", row.waiting_label)) {
                        problems.push(format!("{context}: no blank before Waiting On: {bottom:?}"));
                    }
                    if !(bottom.contains(&title) || bottom.contains(mark)) {
                        problems.push(format!(
                            "{context}: the title neither whole nor cut with {mark:?}: {bottom:?}"
                        ));
                    }
                }
            }
        }
    }
    assert_no_problems(&problems);
}

/// An emoji presentation sequence: U+2B06 U+FE0F, one cell by its scalars'
/// widths but two as drawn, the way Dependabot and gitmoji PR titles start.
const WIDE_EMOJI: &str = "\u{2b06}\u{fe0f}";

/// `WIDE_EMOJI` as drawn in each glyph profile: the ASCII engine draws a
/// non-ASCII grapheme as `?` and keeps its two cells, the second blank.
fn drawn_emoji(glyphs: GlyphProfile) -> &'static str {
    match glyphs {
        GlyphProfile::Unicode => WIDE_EMOJI,
        GlyphProfile::Ascii => "? ",
    }
}

/// The text inside `area`, one line per row and trimmed, as a reader sees it:
/// the cell a wide symbol hides is skipped rather than read as a blank.
fn drawn_lines(buffer: &Buffer, area: Rect) -> Vec<String> {
    (area.top()..area.bottom())
        .map(|y| {
            let mut line = String::new();
            let mut x = area.left();
            while x < area.right() {
                let symbol = buffer.cell((x, y)).map_or(" ", |cell| cell.symbol());
                line.push_str(symbol);
                let cells = u16::try_from(symbol.width()).unwrap_or(1).max(1);
                x = x.saturating_add(cells);
            }
            line.trim().to_owned()
        })
        .collect()
}

/// A title of wide emoji is measured as drawn: whole when it fits, otherwise
/// cut with the profile's ellipsis inside its column, never wider than it and
/// never cut without a sign. Swept over every count of emoji up to the plate's
/// width, after an even and an odd number of narrow cells, so both the exact
/// fit and an emoji straddling the edge are hit.
#[test]
fn a_title_of_wide_emoji_is_cut_by_drawn_width_with_an_ellipsis() {
    let mut problems = Vec::new();
    for size in [MINIMUM_SIZE, DESIGN_SIZE] {
        let harness = surveyed_harness(size);
        let (cols, rows) = size;
        for glyphs in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
            let theme =
                Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
            let mark = ellipsis(glyphs);
            let width = frame::regions(Rect::new(0, 0, cols, rows), &harness.view.layout)
                .list
                .width;
            for (lead, count) in ["", "x"]
                .into_iter()
                .flat_map(|lead| (1..=usize::from(width)).map(move |count| (lead, count)))
            {
                let mut view = harness.view.clone();
                let title = format!("{lead}{}", WIDE_EMOJI.repeat(count));
                for row in &mut view.list.rows {
                    row.title.clone_from(&title);
                }
                let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
                let lines = drawn_lines(&buffer, frame::regions(buffer.area, &view.layout).list);
                let text = lines.join("\n");
                let context = format!("{glyphs:?} {cols}x{rows}, {lead:?} and {count} emoji");
                let whole = format!("{lead}{}", drawn_emoji(glyphs).repeat(count));
                let whole = whole.trim_end();
                for row in &view.list.rows {
                    let first = lines
                        .iter()
                        .position(|l| l.contains(&row.repo))
                        .unwrap_or_else(|| panic!("{context}: no row for {}\n{text}", row.repo));
                    let bottom = &lines[first + 1];
                    if !bottom.ends_with(&format!(" {}", row.waiting_label)) {
                        problems.push(format!("{context}: no blank before Waiting On: {bottom:?}"));
                    }
                    if !(bottom.contains(whole) || bottom.contains(mark)) {
                        problems.push(format!(
                            "{context}: the title neither whole nor cut with {mark:?}: {bottom:?}"
                        ));
                    }
                }
            }
        }
    }
    assert_no_problems(&problems);
}
