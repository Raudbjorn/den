//! Text helpers for templates: wrapping and line lists.

use crepuscularity_tui::{TemplateContext, TemplateValue};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Greedy word wrap to `width` terminal cells (a wide character such as CJK
/// takes two). Words wider than a line are split. Blank text is one empty line
/// (a spacer).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_cells = 0;
    for piece in text
        .split_whitespace()
        .flat_map(|word| split_to_width(word, width))
    {
        let piece_cells = piece.width();
        if !line.is_empty() && line_cells + 1 + piece_cells > width {
            lines.push(std::mem::take(&mut line));
            line_cells = 0;
        }
        if !line.is_empty() {
            line.push(' ');
            line_cells += 1;
        }
        line.push_str(&piece);
        line_cells += piece_cells;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// Splits one word into pieces at most `width` cells wide. A single character
/// wider than `width` still gets a piece of its own, so this always advances.
fn split_to_width(word: &str, width: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut piece_cells = 0;
    for c in word.chars() {
        let cells = c.width().unwrap_or(0);
        if !piece.is_empty() && piece_cells + cells > width {
            pieces.push(std::mem::take(&mut piece));
            piece_cells = 0;
        }
        piece.push(c);
        piece_cells += cells;
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    pieces
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
    /// Checked and fine (success colour); the text carries a glyph too.
    Good,
    /// An input field's text.
    Input,
    /// An input field that has the keyboard.
    InputActive,
    /// A fork's Upstream as the target (rust); the text says "upstream →" too.
    Upstream,
}

pub fn line_context(text: &str, style: LineStyle) -> TemplateContext {
    let mut ctx = TemplateContext::default();
    ctx.set("text", text)
        .set("is_label", style == LineStyle::Label)
        .set("is_strong", style == LineStyle::Strong)
        .set("is_step", style == LineStyle::Step)
        .set("is_caution", style == LineStyle::Caution)
        .set("is_muted", style == LineStyle::Muted)
        .set("is_good", style == LineStyle::Good)
        .set("is_input", style == LineStyle::Input)
        .set("is_input_active", style == LineStyle::InputActive)
        .set("is_upstream", style == LineStyle::Upstream);
    ctx
}

pub fn lines_value(lines: impl IntoIterator<Item = TemplateContext>) -> TemplateValue {
    TemplateValue::List(lines.into_iter().collect())
}
