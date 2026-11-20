//! Seam 2: Issues in the App, with fake ports. Asserts on the ViewModel only.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    app::Key,
    domain::{
        NextStep, WaitingOn,
        facts::{LinkKind, LinkedPrState},
    },
    ui::{Theme, frame},
};
use raudbjorn_tui::profile::{ColorProfile, GlyphProfile, TerminalProfile};
use support::{
    build::{ME, ObservationBuilder, issue},
    harness::Harness,
};

#[test]
fn issues_are_listed_and_a_suppressed_one_names_its_linked_prs_in_the_detail_pane() {
    let mut harness = Harness::with_owners(&["alpha"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .issue(
            issue("alpha/app", 1)
                .assignee(ME)
                .updated("2026-11-19T10:00:00Z"),
        )
        .issue(issue("alpha/app", 2).updated("2026-11-19T09:00:00Z"))
        .issue(
            issue("alpha/app", 3)
                .updated("2026-11-19T08:00:00Z")
                .linked(
                    LinkKind::ManualLink,
                    "elsewhere/lib",
                    77,
                    LinkedPrState::Open,
                )
                .linked(
                    LinkKind::ClosingReference,
                    "alpha/app",
                    4,
                    LinkedPrState::Closed,
                ),
        )
        .build()));
    let view = harness.start().clone();

    // Sorted here: the list's order is the ordering module's business.
    let mut rows: Vec<(String, Option<NextStep>, WaitingOn)> = view
        .list
        .rows
        .iter()
        .map(|r| (r.reference.clone(), r.next_step, r.waiting_on))
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        [
            ("#1".to_owned(), Some(NextStep::Implement), WaitingOn::Me),
            ("#2".to_owned(), Some(NextStep::Triage), WaitingOn::Nobody),
            ("#3".to_owned(), None, WaitingOn::Nobody),
        ]
    );

    let row_of_3 = view
        .list
        .rows
        .iter()
        .position(|r| r.reference == "#3")
        .expect("a row for #3");
    for _ in 0..row_of_3 {
        harness.key(Key::Down);
    }
    let detail = harness.view.detail.content.clone().expect("#3 is selected");
    assert_eq!(detail.reference, "alpha/app#3");
    assert_eq!(detail.next_step, "none");
    assert!(detail.why.contains("elsewhere/lib#77"), "{}", detail.why);
    for line in [
        "linked PR elsewhere/lib#77 (manual link, open)",
        "linked PR alpha/app#4 (closing reference, closed)",
    ] {
        assert!(
            detail.also.iter().any(|a| a.contains(line)),
            "{line:?} in {:?}",
            detail.also
        );
    }
}

/// Room for the list and the detail plate (the frame's design size).
const FRAME_SIZE: (u16, u16) = (160, 45);

#[test]
fn control_characters_in_issue_text_never_reach_the_terminal() {
    // Anyone who can edit an Issue controls its title and label names. ESC, BEL
    // and friends there could drive the user's terminal (clear it, set the
    // clipboard via OSC 52) if they were written out.
    let hostile = "Fix \u{1b}[2J\u{1b}]52;c;cm0gLXJmIH4=\u{7}it\u{9b}31m now";
    let mut harness = Harness::with_owners(&["alpha"]);
    harness.answer(Ok(ObservationBuilder::new()
        .fully_read_owner("alpha")
        .issue(issue("alpha/app", 1).title(hostile).label(hostile))
        .build()));
    let view = harness.start().clone();
    assert!(view.detail.content.is_some(), "the Issue is selected");

    let theme = Theme::new(TerminalProfile::new(
        ColorProfile::TrueColor,
        GlyphProfile::Unicode,
    ))
    .expect("theme");
    let (cols, rows) = FRAME_SIZE;
    let buffer = frame::render(&theme, &view, cols, rows).expect("renders");
    let control: Vec<String> = buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .filter(|symbol| symbol.chars().any(char::is_control))
        .map(|symbol| format!("{symbol:?}"))
        .collect();
    assert!(
        control.is_empty(),
        "control characters in cells: {control:?}"
    );
    // The printable rest of the title is still shown.
    assert!(frame::buffer_text(&buffer).contains("Fix"));
}
