//! `DESIGN.md`'s references resolve (#146): every repo path the page writes
//! in backticks exists, apart from the den kit's planned files (which must
//! not exist yet, so this test says when to drop them from the list), and
//! every ADR and spec the page cites by number has its file named on the
//! page. A renamed or deleted file fails here instead of leaving a dead
//! reference.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

/// Paths the page names that don't exist yet: the den kit (ADR-0006,
/// milestone 3). When the kit lands, remove them here and the "planned"
/// notes on the page.
const PLANNED: [&str; 2] = ["src/ui/kit/", "src/ui/kit/theme.rs"];

/// Where the page lists every cited ADR and spec by path.
const ADR_DIR: &str = "docs/adr/";
const SPEC_DIR: &str = "docs/specs/";
/// Digits in an ADR or spec number (`ADR-0006`, `spec 0003`).
const NUMBER_DIGITS: usize = 4;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn design() -> String {
    std::fs::read_to_string(root().join("DESIGN.md")).expect("DESIGN.md at the repo root")
}

/// The page's inline code spans, in order.
fn code_spans(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// File extensions that make a span without `/` a path too, read from the
/// repo root: a bare `workbench-200x56.png` or `SOURCE.md` must be written
/// in full unless the file really is at the root (`CONTEXT.md`).
const FILE_EXTENSIONS: [&str; 6] = ["md", "rs", "png", "svg", "json", "toml"];

/// A code span that is a repo-relative path: no spaces, every segment
/// letters, digits, `.`, `_` or `-`, and either a `/` in it (a trailing `/`
/// is a directory) or a file extension den's files use. `owner/name`-style
/// placeholders inside longer spans have spaces around them and don't
/// count.
fn is_path(span: &str) -> bool {
    let has_extension = span
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && FILE_EXTENSIONS.contains(&ext));
    (span.contains('/') || has_extension)
        && !span.starts_with('/')
        && span.trim_end_matches('/').split('/').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

/// What on `text` doesn't resolve under `root`: a written path that doesn't
/// exist (or a planned one that now does), or an ADR or spec cited by
/// number without its file named.
fn unresolved(text: &str, root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let paths: Vec<&str> = code_spans(text)
        .into_iter()
        .filter(|s| is_path(s))
        .collect();
    for path in &paths {
        let exists = root.join(path).exists();
        if PLANNED.contains(path) {
            if exists {
                problems.push(format!(
                    "{path} exists now: drop it from PLANNED and the page's \"planned\" notes"
                ));
            }
        } else if !exists {
            problems.push(format!("{path} doesn't exist"));
        }
    }
    for (cited, dir) in [("ADR-", ADR_DIR), ("spec ", SPEC_DIR)] {
        for number in cited_numbers(text, cited) {
            let prefix = format!("{dir}{number}-");
            if !paths.iter().any(|p| p.starts_with(&prefix)) {
                problems.push(format!(
                    "{cited}{number} is cited but no `{prefix}…` path is named"
                ));
            }
        }
    }
    problems
}

/// The four-digit numbers written right after `cited` (`ADR-`, `spec `).
fn cited_numbers(text: &str, cited: &str) -> Vec<String> {
    let mut numbers: Vec<String> = text
        .match_indices(cited)
        .filter_map(|(at, _)| {
            let digits: String = text[at + cited.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            (digits.len() == NUMBER_DIGITS).then_some(digits)
        })
        .collect();
    numbers.sort();
    numbers.dedup();
    numbers
}

#[test]
fn every_reference_on_the_design_page_resolves() {
    let problems = unresolved(&design(), &root());
    assert!(problems.is_empty(), "DESIGN.md: {problems:#?}");
}

/// The page does name what this test checks: its renders, the cited ADRs
/// and specs, and the engine files, so an empty page can't pass vacuously.
#[test]
fn the_page_names_its_renders_adrs_specs_and_engine_files() {
    let text = design();
    let paths: Vec<&str> = code_spans(&text)
        .into_iter()
        .filter(|s| is_path(s))
        .collect();
    for expected in [
        "docs/specs/0003-assets/workbench-160x45.png",
        "docs/specs/0003-assets/workbench-200x56.png",
        "docs/specs/0003-assets/repos-160x45.png",
        "vendor/raudbjorn-tui/src/theme/mod.rs",
        "vendor/raudbjorn-tui/src/profile.rs",
    ] {
        assert!(paths.contains(&expected), "{expected} is named");
    }
    assert!(cited_numbers(&text, "ADR-").contains(&"0006".to_owned()));
    assert!(cited_numbers(&text, "spec ").contains(&"0003".to_owned()));
}

/// The checker itself: a dead path, a bare render name, a short code path
/// and an ADR cited without its file are each reported; resolvable ones
/// aren't.
#[test]
fn the_checker_reports_what_doesnt_resolve() {
    let page = "See `docs/adr/9999-gone.md`, `kit/theme.rs`, `workbench-200x56.png`, \
                `DESIGN.md`, `docs/specs/0003-den-milestone-3.md`, `den ┊ owner/name`. \
                ADR-0001 rules; spec 0003 says so.";
    let problems = unresolved(page, &root());
    assert_eq!(
        problems,
        [
            "docs/adr/9999-gone.md doesn't exist",
            "kit/theme.rs doesn't exist",
            "workbench-200x56.png doesn't exist",
            "ADR-0001 is cited but no `docs/adr/0001-…` path is named",
        ],
        "a bare file name is read from the repo root, so a render must be written in full"
    );
}
