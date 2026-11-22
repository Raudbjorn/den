//! Smoke checks for the launch sheet's rendering: it draws its fields, prompt
//! and command at the design and minimum sizes in Unicode and ASCII, keeps the
//! status and messages when rows are short, and no bear is drawn under it. How
//! it looks is verified by hand (spec, Testing Decisions).

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Event, Key},
    config::Config,
    ui::{Theme, frame},
};
use ratatui::{buffer::Buffer, layout::Rect};
use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};
use std::{collections::BTreeMap, path::PathBuf};
use support::{
    build::{ME, ObservationBuilder, issue, pr},
    harness::Harness,
};

const SIZES: [(u16, u16); 3] = [(160, 45), (200, 56), (80, 24)];

fn sheet_open((cols, rows): (u16, u16)) -> Harness {
    let config = Config {
        owners: vec!["octo".to_owned()],
        main_clone_root: Some(PathBuf::from("/home/me/dev")),
        main_clones: BTreeMap::new(),
        linear: Vec::new(),
        skips: den::domain::Skips::default(),
        screenshot_repos: Vec::new(),
        merge_methods: den::domain::MergeMethods::default(),
        refresh_interval_secs: None,
        legacy_marker_prefixes: Vec::new(),
    };
    let mut harness = Harness::new(Ok(config));
    harness.send(Event::Resized { cols, rows });
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .repo("octo/app")
        .pull_request(pr("octo/app", 7).author(ME).changes_requested())
        .build()));
    harness.start();
    harness.workspace.add_clone(
        "/home/me/dev/app",
        &[("upstream", "https://github.com/octo/app")],
    );
    harness.workspace.set_remote_head(
        "/home/me/dev/app",
        "upstream",
        "feature-7",
        "0000000000000000000000000000000000000007",
    );
    harness.settle(Event::Key(Key::Char('c')));
    harness
}

fn bear_cells(buffer: &Buffer, area: Rect) -> usize {
    let area = area.intersection(buffer.area);
    (area.top()..area.bottom())
        .flat_map(|y| (area.left()..area.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| matches!(buffer[(x, y)].symbol(), "▀" | "▄" | "█"))
        .count()
}

#[test]
fn the_launch_sheet_draws_its_fields_prompt_command_and_keys_at_supported_sizes() {
    for size in SIZES {
        let harness = sheet_open(size);
        let (cols, rows) = size;
        for glyphs in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
            let theme =
                Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
            let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
            let text = frame::buffer_text(&buffer);
            for expected in [
                "L A U N C H   J O B",
                "octo/app#7",
                "Address review",
                "STATUS",
                "PROMPT",
                "/gh-resolve",
                "COMMAND",
                "launch",
                "cancel",
            ] {
                assert!(
                    text.contains(expected),
                    "{size:?} {glyphs:?}: no {expected:?}\n{text}"
                );
            }
            if rows >= 45 {
                for expected in [
                    "MAIN CLONE",
                    "/home/me/dev/app",
                    "REMOTE",
                    "WORKTREE",
                    "free",
                ] {
                    assert!(text.contains(expected), "{size:?}: no {expected:?}\n{text}");
                }
            }
        }
    }
}

#[test]
fn the_launch_sheet_has_a_blank_backdrop_instead_of_clipped_workbench_text() {
    for (cols, rows) in SIZES {
        for harness in [sheet_open((cols, rows)), upstream_sheet_open((cols, rows))] {
            for (color, glyphs) in [
                (ColorProfile::TrueColor, GlyphProfile::Unicode),
                (ColorProfile::Mono, GlyphProfile::Ascii),
            ] {
                let theme = Theme::new(TerminalProfile::new(color, glyphs)).expect("theme");
                let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
                let sheet = frame::overlay_area(&harness.view, buffer.area).expect("the sheet");
                let footer = frame::regions(buffer.area, &harness.view.layout).footer;
                for y in buffer.area.top()..footer.top() {
                    for x in buffer.area.left()..buffer.area.right() {
                        if sheet.contains((x, y).into()) {
                            continue;
                        }
                        let cell = &buffer[(x, y)];
                        assert_eq!(
                            cell.symbol(),
                            " ",
                            "{cols}x{rows} {color:?}/{glyphs:?}: workbench leaks at ({x}, {y})"
                        );
                        assert_eq!(cell.bg, buffer[(0, footer.y)].bg);
                    }
                }
            }
        }
    }
}

#[test]
fn a_refusal_and_the_cursor_show_in_text_not_only_colour() {
    let mut harness = sheet_open((80, 24));
    harness.settle(Event::Key(Key::Char('e')));
    let ascii = Theme::new(TerminalProfile::new(
        ColorProfile::Mono,
        GlyphProfile::Ascii,
    ))
    .expect("theme");
    let text = frame::buffer_text(&frame::render(&ascii, &harness.view, 80, 24).expect("renders"));
    assert!(
        text.contains("pull/7|"),
        "the cursor shows as a glyph\n{text}"
    );

    harness.workspace.make_unwritable("/home/me/dev/app");
    harness.settle(Event::Key(Key::Esc));
    harness.settle(Event::Key(Key::Esc));
    harness.settle(Event::Key(Key::Char('c')));
    let text = frame::buffer_text(&frame::render(&ascii, &harness.view, 80, 24).expect("renders"));
    assert!(text.contains("! Refused"), "{text}");
    assert!(
        text.contains("! den can't write in the Main Clone"),
        "{text}"
    );
}

#[test]
fn no_bear_is_drawn_under_the_launch_sheet() {
    let (cols, rows) = (160, 45);
    let harness = sheet_open((cols, rows));
    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::TrueColor,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let buffer = frame::render(&theme, &harness.view, cols, rows).expect("renders");
    let sheet = frame::overlay_area(&harness.view, buffer.area).expect("the sheet");
    assert_eq!(
        bear_cells(&buffer, sheet),
        0,
        "{}",
        frame::buffer_text(&buffer)
    );
    // #150's backdrop hides the whole workbench, including the uncovered bear.
    let header = Rect::new(0, 0, sheet.x, frame::HEADER_PLATE_ROWS);
    assert_eq!(bear_cells(&buffer, header), 0);
}

/// Prints the sheet (run with `--ignored --nocapture` to look at it).
#[test]
#[ignore = "prints the sheet for a look by eye"]
fn print_the_sheet() {
    for size in SIZES {
        let harness = sheet_open(size);
        let theme = Theme::new(TerminalProfile::new(
            ColorProfile::Mono,
            GlyphProfile::Unicode,
        ))
        .expect("theme");
        let buffer = frame::render(&theme, &harness.view, size.0, size.1).expect("renders");
        println!("{}\n", frame::buffer_text(&buffer));
    }
}

/// #13: a fork whose target is its Upstream, its sheet open on PR 7.
fn upstream_sheet_open((cols, rows): (u16, u16)) -> Harness {
    let config = Config {
        owners: vec!["octo".to_owned()],
        main_clone_root: Some(PathBuf::from("/home/me/dev")),
        main_clones: BTreeMap::new(),
        linear: Vec::new(),
        skips: den::domain::Skips::default(),
        screenshot_repos: Vec::new(),
        merge_methods: den::domain::MergeMethods::default(),
        refresh_interval_secs: None,
        legacy_marker_prefixes: Vec::new(),
    };
    let upstreams = std::sync::Arc::new(support::fakes::FakeUpstreams::holding(
        den::domain::UpstreamTargets::new([support::build::repo_id("octo/app")]),
    ));
    let mut harness =
        Harness::with_stores(Ok(config), support::fakes::FakeCache::default(), upstreams);
    harness.send(Event::Resized { cols, rows });
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .fork("octo/app", Some("up/app"))
        // An Issue, not a PR: since decision b a PR's sheet starts on the
        // PR's own Repo whatever the fork's setting; an Issue's follows it.
        .issue(issue("octo/app", 9).assignee(ME))
        .build()));
    harness.start();
    harness.settle(Event::Key(Key::Char('c')));
    harness
}

#[test]
fn an_upstream_target_reads_as_upstream_in_monochrome_on_the_sheet_and_in_the_sidebar() {
    let (cols, rows) = (160, 45);
    let mut harness = upstream_sheet_open((cols, rows));
    // The ASCII profile turns the arrow into `>`; the word stays.
    for (glyphs, target) in [
        (GlyphProfile::Unicode, "upstream → up/app"),
        (GlyphProfile::Ascii, "upstream > up/app"),
    ] {
        let theme = Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
        let text =
            frame::buffer_text(&frame::render(&theme, &harness.view, cols, rows).expect("renders"));
        assert!(text.contains(target), "{glyphs:?}\n{text}");
    }
    harness.settle(Event::Key(Key::Esc));
    for (glyphs, sidebar_label) in [
        (GlyphProfile::Unicode, "upstream →"),
        (GlyphProfile::Ascii, "upstream >"),
    ] {
        let theme = Theme::new(TerminalProfile::new(ColorProfile::Mono, glyphs)).expect("theme");
        let text =
            frame::buffer_text(&frame::render(&theme, &harness.view, cols, rows).expect("renders"));
        assert!(
            text.lines()
                .any(|l| l.contains("  app") && l.contains(sidebar_label)),
            "{glyphs:?}: no sidebar row with {sidebar_label}\n{text}"
        );
    }
}

#[test]
fn wide_prompts_wrap_and_keep_the_edit_cursor_visible_at_the_minimum_size() {
    use den::app::view_model::Overlay;
    for (profile, cursor_glyph) in [(GlyphProfile::Unicode, "▏"), (GlyphProfile::Ascii, "|")] {
        for count in [40, 400] {
            let mut harness = sheet_open((80, 24));
            let Some(Overlay::LaunchSheet(sheet)) = &mut harness.view.overlay else {
                panic!("sheet");
            };
            let prompt = sheet.prompt.as_mut().expect("prompt");
            prompt.text = "界".repeat(count);
            prompt.cursor = count;
            prompt.editing = true;
            let theme =
                Theme::new(TerminalProfile::new(ColorProfile::Mono, profile)).expect("theme");
            let rendered =
                frame::buffer_text(&frame::render(&theme, &harness.view, 80, 24).expect("render"));
            assert!(rendered.contains(cursor_glyph), "{rendered}");
            if count == 40 && profile == GlyphProfile::Unicode {
                assert_eq!(rendered.matches('界').count(), count, "{rendered}");
            }
        }
    }
}
