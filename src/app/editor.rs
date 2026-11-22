//! The launch sheet's prompt field: text edited in place, with a
//! cursor (a character index). The sheet wraps it over several rows.

use super::Key;
use crate::handoff::MAX_PROMPT_CHARS;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptEditor {
    chars: Vec<char>,
    /// Where the next character goes: 0 ..= len.
    cursor: usize,
}

/// What a key did to the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Changed,
    /// The key isn't an editing key (Enter, Esc, Tab, …).
    NotAnEdit,
}

impl PromptEditor {
    /// A field holding `text`, the cursor at its end.
    pub fn new(text: &str) -> Self {
        let chars: Vec<char> = text
            .chars()
            // Preserve publication blocks; check_prompt rejects overlong input.
            .map(|c| if c == '\r' { ' ' } else { c })
            .collect();
        let cursor = chars.len();
        Self { chars, cursor }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn key(&mut self, key: Key) -> Edit {
        match key {
            Key::Char(c) if !c.is_control() => {
                if self.chars.len() < MAX_PROMPT_CHARS {
                    self.chars.insert(self.cursor, c);
                    self.cursor += 1;
                }
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.chars.remove(self.cursor);
                }
            }
            Key::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                }
            }
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            Key::Home | Key::Up => self.cursor = 0,
            Key::End | Key::Down => self.cursor = self.chars.len(),
            _ => return Edit::NotAnEdit,
        }
        Edit::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::PromptEditor;

    /// den's line breaks survive the field (a resolution comment is posted
    /// verbatim, #78); a carriage return becomes a space.
    #[test]
    fn line_breaks_are_kept_and_carriage_returns_become_spaces() {
        // #111 dropped a carriage return; main (#110) makes it a space, and
        // main's editor wins the merge. Line breaks stay either way: a
        // resolution comment is multi-line Markdown posted verbatim (#78).
        let editor = PromptEditor::new("one\r\ntwo\nthree");
        assert_eq!(editor.text(), "one \ntwo\nthree");
        assert_eq!(editor.cursor(), "one \ntwo\nthree".chars().count());
    }
}
