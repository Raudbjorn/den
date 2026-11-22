//! A Pull Request body's Screenshots state (#79; spec 0004, "Screenshots").
//! den reads the body only to compute this state, and keeps the state, never
//! the body: text written by others never reaches a prompt or the cache.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// The section a Repo with `screenshots = true` expects (case ignored).
const SECTION_TITLE: &str = "screenshots";
/// The line that says a PR changes nothing visible (case and a final full
/// stop ignored).
const NO_VISUAL_CHANGE: &str = "no visual change";
/// A section ends at the next heading of this level or higher (`#`, `##`).
const SECTION_LEVEL: usize = 2;
/// What opens a Markdown image.
const IMAGE_OPEN: &str = "![";
const MIN_FENCE_LENGTH: usize = 3;
const MAX_FENCE_INDENT: usize = 3;

/// What a PR body says about how the change looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Screenshots {
    /// A `## Screenshots` section holding an image.
    Present,
    /// A "No visual change" line.
    NoVisualChange,
    /// Neither.
    Missing,
    /// den didn't read the body: the Repo doesn't ask for screenshots, or the
    /// read failed. Facts cached before #79 load as this too.
    #[default]
    NotRead,
}

impl Screenshots {
    /// The state `body` gives. HTML comments and top-level fenced code examples
    /// never count; an unterminated comment or fence runs to the end of the body.
    pub fn from_body(body: &str) -> Self {
        let text = without_examples(body);
        let labels = reference_labels(&text);
        let mut in_section = false;
        let mut present = false;
        let mut no_visual_change = false;
        for line in text.lines() {
            let line = line.trim();
            if let Some((level, title)) = heading(line) {
                if level <= SECTION_LEVEL {
                    in_section =
                        level == SECTION_LEVEL && title.eq_ignore_ascii_case(SECTION_TITLE);
                }
                continue;
            }
            present |= in_section && has_image(line, &labels);
            no_visual_change |= says_no_visual_change(line);
        }
        if present {
            Self::Present
        } else if no_visual_change {
            Self::NoVisualChange
        } else {
            Self::Missing
        }
    }

    /// Whether this state lets a Pull Request in such a Repo be reviewed and merged.
    pub fn satisfied(self) -> bool {
        matches!(self, Self::Present | Self::NoVisualChange)
    }
}

/// Remove comments and code together: delimiters in either are literal in the other.
/// shortcut: handles top-level fences; use a Markdown parser for nested containers.
fn without_examples(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_comment = false;
    let mut fence = None;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        if !in_comment
            && line.len() - trimmed.len() <= MAX_FENCE_INDENT
            && let Some(marker @ (b'`' | b'~')) = trimmed.bytes().next()
        {
            let length = trimmed.bytes().take_while(|&b| b == marker).count();
            let rest = &trimmed[length..];
            if let Some((opening, minimum)) = fence {
                if marker == opening
                    && length >= minimum
                    && rest.trim_matches([' ', '\t', '\r', '\n']).is_empty()
                {
                    fence = None;
                    continue;
                }
            } else if length >= MIN_FENCE_LENGTH && (marker != b'`' || !rest.contains('`')) {
                fence = Some((marker, length));
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        let mut rest = line;
        while !rest.is_empty() {
            if in_comment {
                let Some(end) = rest.find("-->") else {
                    break;
                };
                rest = &rest[end + "-->".len()..];
                in_comment = false;
            } else if let Some(start) = rest.find("<!--") {
                out.push_str(&rest[..start]);
                rest = &rest[start + "<!--".len()..];
                in_comment = true;
            } else {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// A Markdown ATX heading's level and title: `## Screenshots` → (2, "Screenshots").
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let rest = &line[level..];
    if level == 0 || !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    Some((level, rest.trim().trim_end_matches('#').trim_end()))
}

/// A Markdown image or an HTML `<img>`. A Markdown image is inline
/// (`![alt](url)`) or reference-style (`![alt][label]`, `![label][]`,
/// `![label]`); a reference-style one renders only when `labels` defines its
/// label, so an undefined one never counts.
fn has_image(line: &str, labels: &HashSet<String>) -> bool {
    line.find(IMAGE_OPEN)
        .is_some_and(|at| line[at..].contains("]("))
        || line.to_ascii_lowercase().contains("<img")
        || line
            .match_indices(IMAGE_OPEN)
            .any(|(at, _)| is_defined_reference(&line[at + IMAGE_OPEN.len()..], labels))
}

/// Whether the text after a `![` is a reference-style image whose label
/// `labels` defines. A full reference (`[alt][label]`) names its label; a
/// collapsed (`[label][]`) or shortcut (`[label]`) one uses its alt text.
fn is_defined_reference(after_open: &str, labels: &HashSet<String>) -> bool {
    let Some((alt, rest)) = after_open.split_once(']') else {
        return false;
    };
    let label = match rest.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
        Some((label, _)) if !label.trim().is_empty() => label,
        _ => alt,
    };
    labels.contains(&normalized_label(label))
}

/// The normalized labels of `text`'s link reference definitions
/// (`[label]: url`), wherever in the body they sit.
fn reference_labels(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|line| defined_label(line.trim()))
        .collect()
}

/// `[label]: destination` → the normalized label. A footnote (`[^1]: …`)
/// defines no link.
fn defined_label(line: &str) -> Option<String> {
    let (label, destination) = line.strip_prefix('[')?.split_once("]:")?;
    let label = normalized_label(label);
    let defines_link = !label.is_empty()
        && !label.starts_with('^')
        && !label.contains(['[', ']'])
        && !destination.trim().is_empty();
    defines_link.then_some(label)
}

/// A label as Markdown matches it: case and runs of whitespace ignored.
fn normalized_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn says_no_visual_change(line: &str) -> bool {
    line.trim_end_matches('.')
        .trim()
        .eq_ignore_ascii_case(NO_VISUAL_CHANGE)
}
