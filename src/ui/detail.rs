//! The detail plate: the selected item's Next Step, Waiting On and Explanation.

use super::{
    Theme, UiError, glyphs, templates,
    text::{self, LineStyle},
};
use crate::app::view_model::{Detail, JobActivity, JobMarker};
use crepuscularity_tui::TemplateContext;
use ratatui::{Frame, layout::Rect};

/// Columns lost to the plate's horizontal padding.
const PADDING_COLS: u16 = 4;
/// Rows above the first line: top padding, label, spacer.
const CHROME_ROWS: u16 = 3;
/// The most lines a title may take.
const TITLE_MAX_LINES: usize = 3;

pub fn draw(frame: &mut Frame, theme: &Theme, detail: &Detail, area: Rect) -> Result<(), UiError> {
    let width = usize::from(area.width.saturating_sub(PADDING_COLS));
    let height = usize::from(area.height.saturating_sub(CHROME_ROWS));
    let glyphs = glyphs::for_profile(theme.profile().glyphs);
    let mut lines: Vec<(String, LineStyle)> = Vec::new();
    match &detail.content {
        None => lines.extend(styled(&detail.placeholder, width, LineStyle::Muted)),
        Some(content) => {
            lines.extend(styled(&content.title, width, LineStyle::Strong).take(TITLE_MAX_LINES));
            let mut push = |text: &str, style: LineStyle| lines.extend(styled(text, width, style));
            push(&glyphs.text(&content.reference), LineStyle::Muted);
            push(" ", LineStyle::Plain);
            if let Some(facts) = &content.facts {
                push(
                    &format!("{} {}", glyphs.linear, facts.label),
                    LineStyle::Label,
                );
                for line in &facts.lines {
                    push(line, LineStyle::Plain);
                }
            }
            if let Some(landing) = &content.landing {
                push(
                    &format!("{} {}", glyphs.linear, landing.label),
                    LineStyle::Label,
                );
                for line in &landing.lines {
                    // The list's own glyph, so the two can't drift apart.
                    let glyph = super::list::job_glyph(line.job, None, &glyphs);
                    let style = if line.caution {
                        LineStyle::Caution
                    } else {
                        LineStyle::Plain
                    };
                    push(&glyphs.text(&format!("{glyph} {}", line.text)), style);
                }
            }
            push("NEXT STEP", LineStyle::Label);
            push(&content.next_step, LineStyle::Step);
            push("WAITING ON", LineStyle::Label);
            push(&content.waiting_on, LineStyle::Plain);
            push(&content.waiting_reason, LineStyle::Muted);
            push("WHY", LineStyle::Label);
            push(&content.why, LineStyle::Plain);
            if !content.also.is_empty() {
                push("ALSO", LineStyle::Label);
                for also in &content.also {
                    push(also, LineStyle::Plain);
                }
            }
            if !content.coverage.is_empty() {
                push("INCOMPLETE DATA", LineStyle::Label);
                for note in &content.coverage {
                    push(&format!("{} {note}", glyphs.incomplete), LineStyle::Caution);
                }
            }
            if let Some(job) = &content.job {
                push("JOB", LineStyle::Label);
                let (glyph, style) = match job.marker {
                    JobMarker::Live => (glyphs.live_job, LineStyle::Step),
                    JobMarker::Missing => (glyphs.job_missing, LineStyle::Muted),
                    JobMarker::Unknown => (glyphs.job_unknown, LineStyle::Caution),
                };
                push(&format!("{glyph} {}", job.summary), style);
                if let Some(activity) = &job.activity {
                    let (glyph, style) = match activity.activity {
                        JobActivity::Running => (glyphs.job_running, LineStyle::Step),
                        JobActivity::Idle => (glyphs.job_idle, LineStyle::Plain),
                        JobActivity::Unknown => (glyphs.unattributed, LineStyle::Muted),
                    };
                    push(&format!("{glyph} {}", activity.text), style);
                }
                for line in &job.lines {
                    push(line, LineStyle::Muted);
                }
            }
            if !content.unknowns.is_empty() {
                push("UNKNOWN", LineStyle::Label);
                for unknown in &content.unknowns {
                    push(&format!("? {unknown}"), LineStyle::Muted);
                }
            }
            push("OBSERVED", LineStyle::Label);
            push(&content.observed, LineStyle::Muted);
            push(" ", LineStyle::Plain);
            push(&content.url, LineStyle::Muted);
        }
    }
    let height = height.max(1);
    let pages = lines.len().div_ceil(height).max(1);
    let page = detail
        .page
        .rem_euclid(i64::try_from(pages).unwrap_or(i64::MAX)) as usize;
    let label = if pages > 1 {
        format!("DETAIL {}/{} PgUp/PgDn", page + 1, pages)
    } else {
        detail.label.clone()
    };
    let lines = lines
        .into_iter()
        .skip(page * height)
        .take(height)
        .map(|(text, style)| text::line_context(&text, style));
    let mut ctx = TemplateContext::default();
    ctx.set("detail_label", label.as_str())
        .set("lines", text::lines_value(lines));
    theme.draw(frame, templates::DETAIL, &ctx, area)
}

fn styled(text: &str, width: usize, style: LineStyle) -> impl Iterator<Item = (String, LineStyle)> {
    text::wrap(text, width)
        .into_iter()
        .map(move |line| (line, style))
}
