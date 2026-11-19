//! Text helpers for templates: wrapping and line lists.

use crepuscularity_tui::{TemplateContext, TemplateValue};

/// Greedy word wrap to `width` characters. Words longer than a line are split.
/// Blank text is one empty line (a spacer).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        while word.len() > width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let rest = word.split_off(width);
            lines.push(word.into_iter().collect());
            word = rest;
        }
        let needed = if line.is_empty() {
            word.len()
        } else {
            line.chars().count() + 1 + word.len()
        };
        if needed > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.extend(word);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// A styled line for the `lines` lists in den's templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    Label,
    Strong,
    Step,
    Caution,
    Muted,
    Plain,
}

pub fn line_context(text: &str, style: LineStyle) -> TemplateContext {
    let mut ctx = TemplateContext::default();
    ctx.set("text", text)
        .set("is_label", style == LineStyle::Label)
        .set("is_strong", style == LineStyle::Strong)
        .set("is_step", style == LineStyle::Step)
        .set("is_caution", style == LineStyle::Caution)
        .set("is_muted", style == LineStyle::Muted);
    ctx
}

pub fn lines_value(lines: impl IntoIterator<Item = TemplateContext>) -> TemplateValue {
    TemplateValue::List(lines.into_iter().collect())
}
