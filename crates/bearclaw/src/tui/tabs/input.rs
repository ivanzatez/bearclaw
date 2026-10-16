//! A one-line text input (search, send, a new note's title) that owns every key while open.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::dash::keys::{RenameAction, rename_action};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputResult {
    Editing,
    Submit(String),
    Cancel,
    Quit,
}

/// Keys map exactly like the rename buffer's, phone quirks included (Return as Ctrl-J).
pub struct LineInput {
    pub prompt: String,
    pub buf: String,
}

impl LineInput {
    pub fn new(prompt: &str) -> Self {
        LineInput {
            prompt: prompt.to_string(),
            buf: String::new(),
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> InputResult {
        match rename_action(k) {
            RenameAction::None => InputResult::Editing,
            RenameAction::Insert(c) => {
                self.buf.push(c);
                InputResult::Editing
            }
            RenameAction::Backspace => {
                self.buf.pop();
                InputResult::Editing
            }
            RenameAction::Commit => InputResult::Submit(self.buf.clone()),
            RenameAction::Cancel => InputResult::Cancel,
            RenameAction::Quit => InputResult::Quit,
        }
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let line = Line::from(vec![
            Span::styled(
                format!(" {}: ", self.prompt),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(self.buf.clone()),
            Span::styled("▏", Style::default().fg(Color::Yellow)),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn line_input_owns_command_keys() {
        let mut i = LineInput::new("search");
        for c in ['q', '1', '?'] {
            assert_eq!(i.key(k(KeyCode::Char(c))), InputResult::Editing);
        }
        assert_eq!(i.buf, "q1?");
        assert_eq!(i.key(k(KeyCode::Tab)), InputResult::Editing);
        assert_eq!(i.key(k(KeyCode::Backspace)), InputResult::Editing);
        assert_eq!(i.key(k(KeyCode::Enter)), InputResult::Submit("q1".into()));
        assert_eq!(i.key(k(KeyCode::Esc)), InputResult::Cancel);
        assert_eq!(
            i.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            InputResult::Quit
        );
    }
}
