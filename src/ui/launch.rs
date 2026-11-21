//! The launch sheet's plate: a label column and pre-wrapped text, drawn over
//! the header and body (the footer stays, with the sheet's keys).
//!
//! When the sheet has more lines than rows, sections give way in this order:
//! the fields' notes move onto their value's line, then go, then fields are cut,
//! then the command, then the prompt, then a Request review's reviewers; the
//! Work Item, its status and the Linear writes (#38: one teal line each,
//! above the command, never wrapped) get space before the messages (refusals,
//! evidence, what a launch did).

use super::{
    Theme, UiError, glyphs, templates,
    text::{self, LineStyle},
};
use crate::app::view_model::{LaunchSheet, LaunchStatus, PromptField, SheetField, Tone};
use crepuscularity_tui::{TemplateContext, TemplateValue};
use ratatui::{Frame, layout::Rect};
use unicode_width::UnicodeWidthStr;

/// Columns lost to the plate's horizontal padding.
const PADDING_COLS: u16 = 4;
/// Width of the label column, in columns (the template's `w-[12]`).
pub const LABEL_COLS: u16 = 12;
/// Rows above the first line: top padding, label, spacer.
const CHROME_ROWS: u16 = 3;
/// The most rows the prompt field takes.
const PROMPT_MAX_ROWS: usize = 6;
/// The most rows a Request review's reviewers field takes.
const REVIEWERS_MAX_ROWS: usize = 2;
/// The most rows the command takes.
const COMMAND_MAX_ROWS: usize = 4;
/// The most rows the title takes.
const TITLE_MAX_ROWS: usize = 2;

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
        .map(|p| input_lines("PROMPT", p, width, PROMPT_MAX_ROWS, glyphs))
        .unwrap_or_default();
    let reviewers = sheet
        .reviewers
        .as_ref()
        .map(|r| input_lines("REVIEWERS", r, width, REVIEWERS_MAX_ROWS, glyphs))
        .unwrap_or_default();
    let command: Vec<Line> = sheet
        .command
        .as_ref()
        .map(|c| labelled("COMMAND", c, width, LineStyle::Muted))
        .unwrap_or_default();
    let linear_writes: Vec<Line> = sheet
        .linear_writes
        .iter()
        .enumerate()
        .map(|(i, write)| {
            let label = if i == 0 { "LINEAR" } else { "" };
            (
                label.to_owned(),
                one_line(write, width, glyphs),
                LineStyle::Step,
            )
        })
        .collect();
    let full_fields = field_lines(&sheet.fields, width, glyphs, Notes::OwnLine);
    let inline_fields = field_lines(&sheet.fields, width, glyphs, Notes::Inline);
    let compact_fields = field_lines(&sheet.fields, width, glyphs, Notes::Hidden);

    // Give rows out by importance; spacers only when there is room for them.
    let spacer = || (String::new(), String::new(), LineStyle::Plain);
    let mut left = rows;
    let head = take(&head, usize::MAX, &mut left, width, glyphs);
    let linear_writes = take(&linear_writes, usize::MAX, &mut left, width, glyphs);
    let messages = take(&messages, usize::MAX, &mut left, width, glyphs);
    let reviewers = take(&reviewers, REVIEWERS_MAX_ROWS, &mut left, width, glyphs);
    let prompt = take(&prompt, PROMPT_MAX_ROWS, &mut left, width, glyphs);
    let command = take(&command, COMMAND_MAX_ROWS, &mut left, width, glyphs);
    let fields = if full_fields.len() <= left {
        full_fields
    } else if inline_fields.len() <= left {
        inline_fields
    } else {
        take(&compact_fields, usize::MAX, &mut left, width, glyphs)
    };
    let sections = [
        head,
        fields,
        reviewers,
        prompt,
        linear_writes,
        command,
        messages,
    ];
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

/// `text` on one row of `width` cells, cut with the profile's ellipsis when
/// longer.
fn one_line(text: &str, width: usize, glyphs: glyphs::Glyphs) -> String {
    let rows = text::wrap(text, width);
    if rows.len() <= 1 {
        return rows.into_iter().next().unwrap_or_default();
    }
    // Room for the ellipsis's cells.
    let first = text::wrap(text, width.saturating_sub(text::cells(glyphs.ellipsis)))
        .into_iter()
        .next()
        .unwrap_or_default();
    text::end_with(first.trim_end(), width, glyphs.ellipsis)
}

/// Up to `cap` lines of `section`, out of the `left` rows still free, each
/// at most `width` cells wide.
fn take(
    section: &[Line],
    cap: usize,
    left: &mut usize,
    width: usize,
    glyphs: glyphs::Glyphs,
) -> Vec<Line> {
    let n = section.len().min(cap).min(*left);
    *left -= n;
    cut(section, n, width, glyphs)
}

/// The first `n` lines of `section`, the last ended with the profile's
/// ellipsis when lines were dropped: within `width` cells, so a line that
/// already filled its row is cut to make room rather than wrapped.
fn cut(section: &[Line], n: usize, width: usize, glyphs: glyphs::Glyphs) -> Vec<Line> {
    let mut kept: Vec<Line> = section.iter().take(n).cloned().collect();
    if n < section.len()
        && let Some(last) = kept.last_mut()
    {
        last.1 = text::end_with(last.1.trim_end(), width, glyphs.ellipsis);
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

/// An input field (the prompt, a Request review's reviewers), wrapped by
/// terminal cells, with the cursor drawn while editing, scrolled so the
/// cursor's row shows within `max_rows`.
fn input_lines(
    label: &str,
    prompt: &PromptField,
    width: usize,
    max_rows: usize,
    glyphs: glyphs::Glyphs,
) -> Vec<Line> {
    let mut chars: Vec<String> = prompt.text.chars().map(String::from).collect();
    let cursor = prompt.cursor.min(chars.len());
    if prompt.editing {
        chars.insert(cursor, glyphs.cursor.to_owned());
    }
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut cursor_row = 0;
    for (i, character) in chars.iter().enumerate() {
        let previous_len = row.len();
        row.push_str(character);
        if previous_len > 0 && row.width() > width.max(1) {
            row.truncate(previous_len);
            rows.push(std::mem::take(&mut row));
            row.push_str(character);
        }
        if i == cursor {
            cursor_row = rows.len();
        }
    }
    rows.push(row);
    let first = cursor_row
        .saturating_sub(max_rows.saturating_sub(1))
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
            let label = if i == 0 { label } else { "" };
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
        LaunchStatus::Ready
        | LaunchStatus::LiveJob
        | LaunchStatus::ReopenReady
        | LaunchStatus::ChooseRepo => Tone::Plain,
        LaunchStatus::Inspecting | LaunchStatus::Launching | LaunchStatus::Writing => Tone::Muted,
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

#[cfg(test)]
mod tests {
    use super::*;
    use raudbjorn_tui::profile::GlyphProfile;

    /// #38: each Linear write is one teal line (the Next Step's style),
    /// labelled LINEAR, directly above the Job's command, however long it is.
    #[test]
    fn each_linear_write_is_one_teal_line_above_the_command() {
        let long = format!(
            "Linear: move TEAM-8 from Todo to {}",
            "In Progress ".repeat(20)
        );
        let sheet = LaunchSheet {
            label: "L A U N C H   J O B".to_owned(),
            reference: "TEAM-8".to_owned(),
            title: "Pick me".to_owned(),
            next_step: "Start".to_owned(),
            status: LaunchStatus::Ready,
            headline: "Ready".to_owned(),
            fields: Vec::new(),
            reviewers: None,
            prompt: Some(PromptField {
                text: "/implement TEAM-8".to_owned(),
                cursor: 0,
                editing: false,
            }),
            linear_writes: vec!["Linear: assign TEAM-8 to Me Myself (you)".to_owned(), long],
            command: Some("kitty --detach claude".to_owned()),
            messages: Vec::new(),
        };
        let width = 60;
        let lines = lines(
            &sheet,
            width,
            40,
            glyphs::for_profile(GlyphProfile::Unicode),
        );
        let writes: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, (_, text, _))| text.starts_with("Linear: "))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(writes.len(), 2, "{lines:#?}");
        for &i in &writes {
            assert_eq!(lines[i].2, LineStyle::Step, "{:?}", lines[i]);
            assert!(lines[i].1.width() <= width, "{:?}", lines[i]);
        }
        assert_eq!(lines[writes[0]].0, "LINEAR");
        assert_eq!(writes[1], writes[0] + 1, "one line each");
        assert!(
            lines[writes[1]]
                .1
                .ends_with(glyphs::for_profile(GlyphProfile::Unicode).ellipsis),
            "cut, not wrapped"
        );
        // The command follows (after a spacer when there is room).
        let command = lines
            .iter()
            .position(|(label, _, _)| label == "COMMAND")
            .expect("a command line");
        assert!(command > writes[1], "{lines:#?}");
        assert!(
            lines[writes[1] + 1..command]
                .iter()
                .all(|(label, text, _)| label.is_empty() && text.is_empty()),
            "nothing between the writes and the command: {lines:#?}"
        );
    }

    #[test]
    fn prompt_rows_use_cell_width_for_wrapping_and_cursor_scrolling() {
        for profile in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
            let glyphs = glyphs::for_profile(profile);
            for (text, cursor) in [
                ("界".repeat(40), 40),
                ("界".repeat(40), 30),
                ("界".repeat(400), 400),
                ("e\u{301}界".repeat(20), 60),
            ] {
                let prompt = PromptField {
                    text: text.clone(),
                    cursor,
                    editing: true,
                };
                let rows = input_lines("PROMPT", &prompt, 58, PROMPT_MAX_ROWS, glyphs);
                assert!(rows.iter().all(|(_, row, _)| row.width() <= 58));
                assert!(
                    rows.iter()
                        .take(PROMPT_MAX_ROWS)
                        .any(|(_, row, _)| row.contains(glyphs.cursor))
                );
                if text.chars().count() <= 60 {
                    let displayed = rows
                        .iter()
                        .map(|(_, row, _)| row.as_str())
                        .collect::<String>();
                    assert_eq!(displayed.replace(glyphs.cursor, ""), text);
                }
            }
        }
    }

    /// A section cut to its row cap ends with the profile's ellipsis (`...` in
    /// ASCII, not `…` drawn as `?`), and its last line stays within the text
    /// width even when it already filled its row.
    #[test]
    fn a_cut_section_ends_in_the_profiles_ellipsis_within_its_width() {
        const WIDTH: usize = 20;
        const ROWS: usize = 100;
        // Words exactly as wide as a line, so every wrapped line is full.
        let command = vec!["c".repeat(WIDTH); COMMAND_MAX_ROWS + 2].join(" ");
        let sheet = LaunchSheet {
            label: "LAUNCH".to_owned(),
            reference: "octo/app #7".to_owned(),
            title: "A title".to_owned(),
            next_step: "Review".to_owned(),
            status: LaunchStatus::Ready,
            headline: "Ready to launch".to_owned(),
            fields: Vec::new(),
            prompt: None,
            reviewers: None,
            linear_writes: Vec::new(),
            command: Some(command),
            messages: Vec::new(),
        };
        for profile in [GlyphProfile::Unicode, GlyphProfile::Ascii] {
            let glyphs = glyphs::for_profile(profile);
            let lines = lines(&sheet, WIDTH, ROWS, glyphs);
            let command: Vec<&str> = lines
                .iter()
                .filter(|(_, _, style)| *style == LineStyle::Muted)
                .map(|(_, text, _)| text.as_str())
                .collect();
            assert_eq!(command.len(), COMMAND_MAX_ROWS, "{profile:?}: {lines:#?}");
            let last = command.last().copied().unwrap_or_default();
            assert!(last.ends_with(glyphs.ellipsis), "{profile:?}: {last:?}");
            for (_, text, _) in &lines {
                assert!(text::cells(text) <= WIDTH, "{profile:?}: {text:?}");
                if profile == GlyphProfile::Ascii {
                    assert!(text.is_ascii(), "{profile:?}: {text:?}");
                }
            }
        }
    }
}
