//! Edit a text file inside the dashboard: notes, brain notes, the config.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};
use tui_textarea::TextArea;

use crate::core::tools::tildify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorOutcome {
    None,
    /// Saved (when changed) and closed. A failed save keeps the editor open instead.
    Closed,
}

pub struct Editor {
    path: PathBuf,
    text: TextArea<'static>,
    /// The file ended with a newline: keep it that way on save.
    trailing_newline: bool,
    dirty: bool,
    error: Option<String>,
}

impl Editor {
    /// A missing file opens empty and is created on the first save.
    pub fn open(path: PathBuf) -> std::io::Result<Editor> {
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let lines: Vec<String> = body.lines().map(str::to_string).collect();
        Ok(Editor {
            path,
            text: TextArea::new(if lines.is_empty() {
                vec![String::new()]
            } else {
                lines
            }),
            trailing_newline: body.ends_with('\n'),
            dirty: false,
            error: None,
        })
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn save(&mut self) -> Result<(), String> {
        let mut body = self.text.lines().join("\n");
        if self.trailing_newline {
            body.push('\n');
        }
        let result = (|| {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&self.path, body)
        })()
        .map_err(|e| format!("{}: {e}", self.path.display()));
        match &result {
            Ok(()) => {
                self.dirty = false;
                self.error = None;
            }
            Err(e) => self.error = Some(e.clone()),
        }
        result
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let name = tildify(&self.path.display().to_string());
        let mark = if self.dirty { " •" } else { "" };
        let hint = match &self.error {
            Some(e) => format!(" ✕ {e} "),
            None => " ctrl+s save · esc save & close ".into(),
        };
        self.text.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(format!(" {name}{mark} "))
                .title_bottom(Line::styled(hint, Style::default().fg(Color::DarkGray))),
        );
        self.text.set_cursor_line_style(Style::default());
        f.render_widget(&self.text, area);
    }

    /// Esc and Ctrl+C save (when changed) and close: nothing typed is ever dropped silently.
    pub fn key(&mut self, k: KeyEvent) -> EditorOutcome {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => return self.close(),
            KeyCode::Char('c') if ctrl => return self.close(),
            KeyCode::Char('s') if ctrl => {
                let _ = self.save();
                return EditorOutcome::None;
            }
            _ => {}
        }
        if self.text.input(k) {
            self.dirty = true;
        }
        EditorOutcome::None
    }

    fn close(&mut self) -> EditorOutcome {
        if !self.dirty {
            return EditorOutcome::Closed;
        }
        match self.save() {
            Ok(()) => EditorOutcome::Closed,
            // Stay open: closing now would throw the text away.
            Err(_) => EditorOutcome::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn editor_saves_on_escape() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.md");
        std::fs::write(&p, "a\n").unwrap();
        let mut e = Editor::open(p.clone()).unwrap();
        assert!(!e.dirty());
        e.key(k(KeyCode::Char('b')));
        e.key(k(KeyCode::Char('c')));
        assert!(e.dirty());
        assert_eq!(e.key(k(KeyCode::Esc)), EditorOutcome::Closed);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "bca\n");
    }

    #[test]
    fn editor_creates_a_missing_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/new.txt");
        let mut e = Editor::open(p.clone()).unwrap();
        e.key(k(KeyCode::Char('x')));
        let save = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(e.key(save), EditorOutcome::None);
        assert!(!e.dirty());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "x");
    }
}
