//! Text helpers for templates: measuring, cutting, wrapping and line lists.
//!
//! Text is measured and cut the way the engine draws it: ratatui puts one
//! grapheme in a cell symbol and gives it the `unicode-width` width of the
//! whole grapheme. Adding up per-`char` widths instead undercounts an emoji
//! presentation sequence such as `⬆️` (U+2B06 U+FE0F: one cell by its scalars,
//! two as drawn).

use crepuscularity_tui::{TemplateContext, TemplateValue};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The terminal cells `text` takes as drawn: the sum of its graphemes' widths.
pub fn cells(text: &str) -> usize {
    text.graphemes(true).map(UnicodeWidthStr::width).sum()
}

/// The longest prefix of `text` at most `width` cells wide, cut between
/// graphemes (a grapheme that would straddle the edge is left out).
pub fn prefix(text: &str, width: usize) -> &str {
    let mut taken = 0;
    for (start, grapheme) in text.grapheme_indices(true) {
        taken += grapheme.width();
        if taken > width {
            return &text[..start];
        }
    }
    text
}

/// `text` in at most `width` cells: whole when it fits, otherwise cut and
/// ended with `ellipsis` (see [`end_with`]), so a cut is never silent.
pub fn fit(text: &str, width: usize, ellipsis: &str) -> String {
    if cells(text) <= width {
        return text.to_owned();
    }
    end_with(text, width, ellipsis)
}

/// `text` ended with `ellipsis` in at most `width` cells, for text known to go
/// on (lines were dropped after it): cut as far as the ellipsis needs. Narrower
/// than the ellipsis itself, it is as much of the ellipsis as fits.
pub fn end_with(text: &str, width: usize, ellipsis: &str) -> String {
    let kept = prefix(text, width.saturating_sub(cells(ellipsis)));
    let mut out = kept.to_owned();
    out.push_str(prefix(ellipsis, width.saturating_sub(cells(kept))));
    out
}

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
        let piece_cells = cells(piece);
        if !line.is_empty() && line_cells + 1 + piece_cells > width {
            lines.push(std::mem::take(&mut line));
            line_cells = 0;
        }
        if !line.is_empty() {
            line.push(' ');
            line_cells += 1;
        }
        line.push_str(piece);
        line_cells += piece_cells;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// Splits one word into pieces at most `width` cells wide, between
/// graphemes. A single grapheme wider than `width` still gets a piece of its
/// own, so this always advances.
fn split_to_width(word: &str, width: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = word;
    while let Some(first) = rest.graphemes(true).next() {
        let piece = match prefix(rest, width) {
            "" => first,
            fits => fits,
        };
        pieces.push(piece);
        rest = &rest[piece.len()..];
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
    /// A fork's Upstream as the target: strong text, never rust (#157); the
    /// text says "upstream →".
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
