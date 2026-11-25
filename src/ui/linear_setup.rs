//! The connection sheet renders only safe metadata and masked input.
use super::{Theme, glyphs};
use crate::{linear::setup::Setup, palette::oxide_for};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use raudbjorn_tui::profile::GlyphProfile;

pub fn draw(frame: &mut Frame, theme: &Theme, setup: &Setup) {
    let Some((lines, selected)) = setup.lines() else {
        return;
    };
    draw_lines(frame, theme, &lines, selected);
}

fn draw_lines(frame: &mut Frame, theme: &Theme, lines: &[String], selected: usize) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(84);
    let height = screen.height.saturating_sub(2).min(22);
    if width < 4 || height < 4 {
        return;
    }
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    let palette = oxide_for(theme.profile().color);
    let marks = glyphs::for_profile(theme.profile().glyphs);
    let visible = usize::from(height.saturating_sub(4));
    let start = selected.saturating_sub(visible.saturating_sub(1));
    let text = lines
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, line)| {
            let marker = if index == selected && selected > 0 {
                "> "
            } else {
                "  "
            };
            let safe: String = line
                .chars()
                .map(|c| {
                    if c.is_control() {
                        ' '
                    } else if theme.profile().glyphs == GlyphProfile::Ascii && !c.is_ascii() {
                        '?'
                    } else {
                        c
                    }
                })
                .collect();
            format!(
                "{marker}{}",
                marks.fit(&safe, usize::from(width.saturating_sub(4)))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let border = if theme.profile().glyphs == GlyphProfile::Ascii {
        BorderType::Plain
    } else {
        BorderType::Rounded
    };
    let mut block = Block::default()
        .title(" LINEAR CONNECTION ")
        .title_bottom(" Up/Down select | Enter continue | Esc close ")
        .borders(Borders::ALL)
        .border_type(border)
        .border_style(Style::default().fg(palette.border_strong));
    if theme.profile().glyphs == GlyphProfile::Ascii {
        block = block.border_set(ratatui::symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        });
    }
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(palette.text).bg(palette.surface_1))
            .block(block),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use raudbjorn_tui::profile::{ColorProfile, TerminalProfile};
    #[test]
    fn setup_sheet_respects_ascii_and_small_terminals() {
        let theme = Theme::new(TerminalProfile {
            color: ColorProfile::Ansi16,
            glyphs: GlyphProfile::Ascii,
        })
        .unwrap();
        for (width, height) in [(100, 30), (30, 8), (3, 2)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    draw_lines(
                        frame,
                        &theme,
                        &[
                            "Workspace: Björn\u{1b}[31m".into(),
                            "API key: ********".into(),
                        ],
                        1,
                    )
                })
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.is_ascii(), "{text}");
            assert!(!text.contains('\u{1b}'));
        }
    }
}
