//! The launch sheet's plate: a label column and pre-wrapped text, drawn over
//! the header and body (the footer stays, with the sheet's keys).
//!
//! When the sheet has more lines than rows, sections give way in this order:
//! the fields' notes move onto their value's line, then go, then fields are cut,
//! then the command, then the prompt; the Work Item, its status and the messages
//! (refusals, what a launch did) stay.

use super::{
    Theme, UiError, glyphs, templates,
    text::{self, LineStyle},
};
use crate::app::view_model::{LaunchSheet, LaunchStatus, PromptField, SheetField, Tone};
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{Frame, layout::Rect};

/// Columns lost to the plate's horizontal padding.
const PADDING_COLS: u16 = 4;
/// Width of the label column, in columns (the template's `w-[12]`).
pub const LABEL_COLS: u16 = 12;
/// Rows above the first line: top padding, label, spacer.
const CHROME_ROWS: u16 = 3;
/// The most rows the prompt field takes.
const PROMPT_MAX_ROWS: usize = 6;
/// The most rows the command takes.
const COMMAND_MAX_ROWS: usize = 4;
/// The most rows the title takes.
const TITLE_MAX_ROWS: usize = 2;
/// Shown at the end of a cut-off field.
const ELLIPSIS: &str = "…";

/// One line of the sheet: label column, text, style.
type Line = (String, String, LineStyle);

pub fn draw(
    frame: &mut Frame,
    theme: &Theme,
    sheet: &LaunchSheet,
    area: Rect,
) -> Result<(), UiError> {
    let width = usize::from(area.width.saturating_sub(PADDING_COLS + LABEL_COLS)).max(1);
    let rows = usize::from(area.height.saturating_sub(CHROME_ROWS));
    let lines = lines(
        sheet,
        width,
        rows,
        glyphs::for_profile(theme.profile().glyphs),
    );
    let items = lines.into_iter().map(|(label, text, style)| {
        let mut ctx = text::line_context(&text, style);
        ctx.set("label", label);
        ctx
    });
    let mut ctx = TemplateContext::default();
    ctx.set("sheet_label", sheet.label.as_str())
        .set("lines", TemplateValue::List(items.collect()));
    theme.draw(frame, templates::LAUNCH_SHEET, &ctx, area)
}

/// The sheet's lines for a text width of `width` and at most `rows` rows.
pub fn lines(sheet: &LaunchSheet, width: usize, rows: usize, glyphs: glyphs::Glyphs) -> Vec<Line> {
    let mut head = labelled("WORK ITEM", &sheet.reference, width, LineStyle::Strong);
    head.extend(
        labelled("", &sheet.title, width, LineStyle::Plain)
            .into_iter()
            .take(TITLE_MAX_ROWS),
    );
    head.extend(labelled(
        "NEXT STEP",
        &sheet.next_step,
        width,
        LineStyle::Step,
    ));
    let (glyph, style) = tone(status_tone(sheet.status), glyphs);
    head.extend(labelled(
        "STATUS",
        &prefixed(glyph, &sheet.headline),
        width,
        style,
    ));

    let messages: Vec<Line> = sheet
        .messages
        .iter()
        .flat_map(|m| {
            let (glyph, style) = tone(m.tone, glyphs);
            labelled("", &prefixed(glyph, &m.text), width, style)
        })
        .collect();
    let prompt = sheet
        .prompt
        .as_ref()
        .map(|p| prompt_lines(p, width, glyphs))
        .unwrap_or_default();
    let command: Vec<Line> = sheet
        .command
        .as_ref()
        .map(|c| labelled("COMMAND", c, width, LineStyle::Muted))
        .unwrap_or_default();
    let full_fields = field_lines(&sheet.fields, width, glyphs, Notes::OwnLine);
    let inline_fields = field_lines(&sheet.fields, width, glyphs, Notes::Inline);
    let compact_fields = field_lines(&sheet.fields, width, glyphs, Notes::Hidden);

    // Give rows out by importance; spacers only when there is room for them.
    let spacer = || (String::new(), String::new(), LineStyle::Plain);
    let mut left = rows;
    let head = take(&head, usize::MAX, &mut left);
    let messages = take(&messages, usize::MAX, &mut left);
    let prompt = take(&prompt, PROMPT_MAX_ROWS, &mut left);
    let command = take(&command, COMMAND_MAX_ROWS, &mut left);
    let fields = if full_fields.len() <= left {
        full_fields
    } else if inline_fields.len() <= left {
        inline_fields
    } else {
        take(&compact_fields, usize::MAX, &mut left)
    };
    let sections = [head, fields, prompt, command, messages];
    let filled: usize = sections.iter().map(Vec::len).sum();
    let gaps = sections
        .iter()
        .filter(|s| !s.is_empty())
        .count()
        .saturating_sub(1);
    let with_spacers = filled + gaps <= rows;
    let mut out = Vec::with_capacity(rows);
    for section in sections.into_iter().filter(|s| !s.is_empty()) {
        if with_spacers && !out.is_empty() {
            out.push(spacer());
        }
        out.extend(section);
    }
    out
}

/// Up to `cap` lines of `section`, out of the `left` rows still free.
fn take(section: &[Line], cap: usize, left: &mut usize) -> Vec<Line> {
    let n = section.len().min(cap).min(*left);
    *left -= n;
    cut(section, n)
}

/// The first `n` lines of `section`, the last marked as cut when lines were dropped.
fn cut(section: &[Line], n: usize) -> Vec<Line> {
    let mut kept: Vec<Line> = section.iter().take(n).cloned().collect();
    if n < section.len()
        && let Some(last) = kept.last_mut()
    {
        last.1 = format!("{}{ELLIPSIS}", last.1.trim_end());
    }
    kept
}

/// Where a field's note goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notes {
    /// On its own line under the value, in the field's tone.
    OwnLine,
    /// After the value, on the value's line(s).
    Inline,
    Hidden,
}

fn field_lines(
    fields: &[SheetField],
    width: usize,
    glyphs: glyphs::Glyphs,
    notes: Notes,
) -> Vec<Line> {
    let mut lines = Vec::new();
    for field in fields {
        let (glyph, style) = tone(field.tone, glyphs);
        // An Upstream target's value is rust too; other values are plain.
        let value_style = if style == LineStyle::Upstream {
            style
        } else {
            LineStyle::Plain
        };
        match (&field.note, notes) {
            (Some(note), Notes::OwnLine) => {
                lines.extend(labelled(&field.label, &field.value, width, value_style));
                lines.extend(labelled("", &prefixed(glyph, note), width, style));
            }
            (Some(note), Notes::Inline) => {
                let style = if matches!(style, LineStyle::Caution | LineStyle::Upstream) {
                    style
                } else {
                    LineStyle::Plain
                };
                let text = format!("{} · {}", field.value, prefixed(glyph, note));
                lines.extend(labelled(&field.label, &text, width, style));
            }
            _ => lines.extend(labelled(&field.label, &field.value, width, value_style)),
        }
    }
    lines
}

/// The prompt, wrapped by characters (so the cursor maps to a cell), with the
/// cursor drawn while editing, scrolled so the cursor's row shows.
fn prompt_lines(prompt: &PromptField, width: usize, glyphs: glyphs::Glyphs) -> Vec<Line> {
    let mut chars: Vec<String> = prompt.text.chars().map(String::from).collect();
    let cursor = prompt.cursor.min(chars.len());
    if prompt.editing {
        chars.insert(cursor, glyphs.cursor.to_owned());
    }
    let rows: Vec<String> = if chars.is_empty() {
        vec![String::new()]
    } else {
        chars.chunks(width).map(|c| c.concat()).collect()
    };
    let cursor_row = cursor / width;
    let first = cursor_row
        .saturating_sub(PROMPT_MAX_ROWS - 1)
        .min(rows.len());
    let first = if prompt.editing { first } else { 0 };
    let style = if prompt.editing {
        LineStyle::InputActive
    } else {
        LineStyle::Input
    };
    rows.into_iter()
        .skip(first)
        .enumerate()
        .map(|(i, row)| {
            let label = if i == 0 { "PROMPT" } else { "" };
            (label.to_owned(), row, style)
        })
        .collect()
}

/// Word-wrapped `text`, the label on its first line only.
fn labelled(label: &str, text: &str, width: usize, style: LineStyle) -> Vec<Line> {
    text::wrap(text, width)
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let label = if i == 0 { label } else { "" };
            (label.to_owned(), line, style)
        })
        .collect()
}

fn prefixed(glyph: Option<&str>, text: &str) -> String {
    match glyph {
        Some(glyph) => format!("{glyph} {text}"),
        None => text.to_owned(),
    }
}

fn status_tone(status: LaunchStatus) -> Tone {
    match status {
        LaunchStatus::Launched | LaunchStatus::Reopened => Tone::Good,
        LaunchStatus::Refused
        | LaunchStatus::Failed
        | LaunchStatus::HeadMoved
        | LaunchStatus::JobUnknown => Tone::Caution,
        LaunchStatus::Ready | LaunchStatus::LiveJob | LaunchStatus::ReopenReady => Tone::Plain,
        LaunchStatus::Inspecting | LaunchStatus::Launching => Tone::Muted,
    }
}

/// The glyph and style for a tone: colour never carries a meaning alone.
fn tone(tone: Tone, glyphs: glyphs::Glyphs) -> (Option<&'static str>, LineStyle) {
    match tone {
        Tone::Good => (Some(glyphs.complete), LineStyle::Good),
        Tone::Caution => (Some(glyphs.incomplete), LineStyle::Caution),
        Tone::Muted => (None, LineStyle::Muted),
        Tone::Plain => (None, LineStyle::Plain),
        // The "upstream →" label is in the text itself.
        Tone::Upstream => (None, LineStyle::Upstream),
    }
}
