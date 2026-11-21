//! A Pull Request body's Screenshots state (#79; spec 0004, "Screenshots").
//! den reads the body only to compute this state, and keeps the state, never
//! the body: text written by others never reaches a prompt or the cache.

use serde::{Deserialize, Serialize};

/// The section a Repo with `screenshots = true` expects (case ignored).
const SECTION_TITLE: &str = "screenshots";
/// The line that says a PR changes nothing visible (case and a final full
/// stop ignored).
const NO_VISUAL_CHANGE: &str = "no visual change";
/// A section ends at the next heading of this level or higher (`#`, `##`).
const SECTION_LEVEL: usize = 2;

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
    /// The state `body` gives. HTML comments are ignored, so a PR template's
    /// commented-out example or hint never counts; an unterminated comment
    /// runs to the end of the body, as it does when GitHub renders it.
    pub fn from_body(body: &str) -> Self {
        let text = without_html_comments(body);
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
            present |= in_section && has_image(line);
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

/// `body` without `<!-- … -->` comments.
fn without_html_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + "-->".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
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

/// A Markdown image (`![alt](url)`) or an HTML `<img>`.
fn has_image(line: &str) -> bool {
    line.find("![").is_some_and(|at| line[at..].contains("]("))
        || line.to_ascii_lowercase().contains("<img")
}

fn says_no_visual_change(line: &str) -> bool {
    line.trim_end_matches('.')
        .trim()
        .eq_ignore_ascii_case(NO_VISUAL_CHANGE)
}
