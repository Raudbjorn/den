//! Seam 2: the responsive layout (#10). The App learns the terminal's size from
//! `Event::Resized`; the ViewModel says which plates exist and how wide they are,
//! and `Enter` opens the detail as a sheet when there is no room for the pane.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::{Event, Key, view_model::Overlay},
    layout::{DESIGN_COLS, MIN_COLS, MIN_ROWS, RAIL_MAX_COLS, RAIL_MIN_COLS, WIDE_LAYOUT_MIN_COLS},
};
use support::{
    build::{ObservationBuilder, now, pr},
    harness::Harness,
};

/// den, surveyed, with three PRs, in a terminal of `cols` x `rows`.
fn surveyed_at(cols: u16, rows: u16) -> Harness {
    let mut harness = Harness::with_owners(&["octo"]);
    harness.answer(Ok(ObservationBuilder::new()
        .owner("octo")
        .pull_request(pr("octo/one", 1))
        .pull_request(pr("octo/two", 2))
        .pull_request(pr("octo/three", 3))
        .build()));
    harness.send(Event::Resized { cols, rows });
    harness.start();
    harness
}

fn resize(harness: &mut Harness, cols: u16, rows: u16) {
    harness.send(Event::Resized { cols, rows });
}

fn hint_keys(harness: &Harness) -> Vec<String> {
    harness
        .view
        .footer
        .hints
        .iter()
        .map(|h| h.key.clone())
        .collect()
}

#[test]
fn below_the_wide_threshold_there_is_no_detail_pane_and_enter_opens_the_detail_sheet() {
    let mut harness = surveyed_at(WIDE_LAYOUT_MIN_COLS - 1, MIN_ROWS);
    assert_eq!(harness.view.layout.detail_pane_cols, None);
    assert_eq!(harness.view.overlay, None);
    assert!(harness.view.list.selected.is_some());

    let effects = harness.key(Key::Enter);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(harness.view.overlay, Some(Overlay::DetailSheet));
    // The sheet shows the selected item's detail.
    let detail = harness
        .view
        .detail
        .content
        .as_ref()
        .expect("detail content");
    assert!(detail.reference.contains("octo/"), "{detail:?}");
}

#[test]
fn from_the_wide_threshold_up_the_detail_is_a_pane_and_enter_opens_no_sheet() {
    let mut harness = surveyed_at(WIDE_LAYOUT_MIN_COLS, MIN_ROWS);
    assert!(harness.view.layout.detail_pane_cols.is_some());

    harness.key(Key::Enter);

    assert_eq!(harness.view.overlay, None);
}

#[test]
fn esc_or_enter_closes_the_detail_sheet() {
    for closing in [Key::Esc, Key::Enter] {
        let mut harness = surveyed_at(MIN_COLS, MIN_ROWS);
        harness.key(Key::Enter);
        assert_eq!(harness.view.overlay, Some(Overlay::DetailSheet));

        let effects = harness.key(closing);

        assert!(effects.is_empty(), "{closing:?}: {effects:?}");
        assert_eq!(harness.view.overlay, None, "{closing:?}");
    }
}

#[test]
fn the_detail_sheet_follows_the_selection() {
    let mut harness = surveyed_at(MIN_COLS, MIN_ROWS);
    harness.key(Key::Enter);
    let first = harness.view.detail.content.clone().expect("detail");

    harness.key(Key::Down);

    assert_eq!(harness.view.overlay, Some(Overlay::DetailSheet));
    assert_eq!(harness.view.list.selected, Some(1));
    let second = harness.view.detail.content.clone().expect("detail");
    assert_ne!(first.reference, second.reference);
}

#[test]
fn widening_the_terminal_closes_the_sheet_because_the_pane_shows_the_detail() {
    let mut harness = surveyed_at(MIN_COLS, MIN_ROWS);
    harness.key(Key::Enter);

    resize(&mut harness, DESIGN_COLS, MIN_ROWS);

    assert_eq!(harness.view.overlay, None);
    assert!(harness.view.layout.detail_pane_cols.is_some());
}

#[test]
fn enter_with_nothing_selected_opens_no_sheet() {
    let mut harness = Harness::with_owners(&["octo"]);
    harness.answer(Ok(ObservationBuilder::new().owner("octo").build()));
    resize(&mut harness, MIN_COLS, MIN_ROWS);
    harness.start();
    assert!(harness.view.list.rows.is_empty());

    harness.key(Key::Enter);

    assert_eq!(harness.view.overlay, None);
}

#[test]
fn q_still_quits_with_the_sheet_open() {
    let mut harness = surveyed_at(MIN_COLS, MIN_ROWS);
    harness.key(Key::Enter);

    let effects = harness.key(Key::Char('q'));

    assert_eq!(effects, [den::app::Effect::Quit]);
}

#[test]
fn the_sidebar_is_22_percent_of_the_width_clamped_to_30_to_40_columns() {
    let mut harness = surveyed_at(DESIGN_COLS, MIN_ROWS);
    for (cols, rail) in [
        (MIN_COLS, RAIL_MIN_COLS),
        (WIDE_LAYOUT_MIN_COLS, RAIL_MIN_COLS),
        (140, 30),
        (DESIGN_COLS, 35),
        (180, 39),
        (250, RAIL_MAX_COLS),
    ] {
        resize(&mut harness, cols, MIN_ROWS);
        assert_eq!(harness.view.layout.rail_cols, rail, "at {cols} columns");
    }
}

#[test]
fn before_the_terminal_reports_a_size_den_assumes_its_design_width() {
    let harness = Harness::with_owners(&["octo"]);
    assert_eq!(harness.view.layout.size.cols, DESIGN_COLS);
    assert!(harness.view.layout.detail_pane_cols.is_some());
}

#[test]
fn a_terminal_smaller_than_80_by_24_is_flagged_and_80_by_24_is_not() {
    let mut harness = surveyed_at(MIN_COLS, MIN_ROWS);
    assert!(!harness.view.layout.undersized);
    for (cols, rows) in [(MIN_COLS - 1, MIN_ROWS), (MIN_COLS, MIN_ROWS - 1)] {
        resize(&mut harness, cols, rows);
        assert!(harness.view.layout.undersized, "{cols}x{rows}");
    }
}

#[test]
fn the_footer_offers_enter_when_narrow_and_esc_while_the_sheet_is_open() {
    let mut harness = surveyed_at(MIN_COLS, MIN_ROWS);
    assert!(hint_keys(&harness).contains(&"⏎".to_owned()));

    harness.key(Key::Enter);
    let keys = hint_keys(&harness);
    assert!(keys.contains(&"esc".to_owned()), "{keys:?}");
    assert!(!keys.contains(&"⏎".to_owned()), "{keys:?}");

    resize(&mut harness, DESIGN_COLS, MIN_ROWS);
    let keys = hint_keys(&harness);
    assert!(!keys.contains(&"⏎".to_owned()), "{keys:?}");
    assert!(!keys.contains(&"esc".to_owned()), "{keys:?}");
}

#[test]
fn the_splash_shows_only_while_the_first_survey_runs() {
    let mut harness = Harness::with_owners(&["octo"]);
    assert!(!harness.view.splash, "before den starts");

    // Started, survey requested but not answered yet.
    harness.send(Event::Started { now: now() });
    assert!(harness.view.splash);

    let mut harness = surveyed_at(DESIGN_COLS, MIN_ROWS);
    assert!(!harness.view.splash, "after the survey");
    harness.key(Key::Down);
    assert!(!harness.view.splash);
}

#[test]
fn no_splash_without_a_usable_config() {
    let mut harness = Harness::new(Err(den::config::ConfigError::Missing {
        path: "/nowhere/config.toml".into(),
    }));
    harness.send(Event::Started { now: now() });
    assert!(!harness.view.splash);
}
