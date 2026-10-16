//! A scrollable, wrapped text view: a note, a task, a transcript.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthChar;

pub struct Pager {
    title: String,
    text: String,
    /// First visible wrapped line; clamped on every draw.
    scroll: usize,
    /// The text area's height on the last draw — what a page is.
    page: usize,
    hint: String,
}

impl Pager {
    pub fn new(title: String, text: String) -> Self {
        Pager {
            title,
            text,
            scroll: 0,
            page: 1,
            hint: " esc back · j/k scroll · g/G top/end ".into(),
        }
    }

    /// Replace the bottom-border key hint.
    pub fn with_hint(mut self, hint: &str) -> Self {
        self.hint = format!(" {hint} ");
        self
    }

    /// New text, same scroll position (clamped on the next draw).
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(format!(" {} ", self.title))
            .title_bottom(Line::styled(
                self.hint.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let lines = wrap(&self.text, inner.width as usize);
        self.page = (inner.height as usize).max(1);
        self.scroll = self.scroll.min(lines.len().saturating_sub(self.page));
        let shown: Vec<Line> = lines
            .into_iter()
            .skip(self.scroll)
            .take(self.page)
            .map(Line::from)
            .collect();
        f.render_widget(Paragraph::new(shown), inner);
    }

    /// Scroll on the pager's keys. False = close it (Esc, `q`, ←, Backspace).
    pub fn key(&mut self, k: KeyEvent) -> bool {
        if k.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Backspace => return false,
            KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.scroll = self.scroll.saturating_add(self.page)
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.page),
            KeyCode::Char('g') | KeyCode::Home => self.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.scroll = usize::MAX,
            _ => {}
        }
        true
    }
}

/// Word-wrap `text` to `width` columns; a word longer than the width is cut. Blank lines stay.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.lines() {
        let mut line = String::new();
        let mut used = 0;
        for word in raw.split(' ') {
            let w: usize = word.chars().map(|c| c.width().unwrap_or(0)).sum();
            let gap = usize::from(!line.is_empty());
            if used + gap + w <= width {
                if gap == 1 {
                    line.push(' ');
                }
                line.push_str(word);
                used += gap + w;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
                used = 0;
            }
            for c in word.chars() {
                let cw = c.width().unwrap_or(0);
                if used + cw > width {
                    out.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(c);
                used += cw;
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn screen(p: &mut Pager, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| p.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn pager_scrolls_and_closes() {
        let text: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        let mut p = Pager::new("Note".into(), text);
        let first = screen(&mut p, 40, 10).join("\n");
        assert!(
            first.contains("line 1 ") && first.contains("Note"),
            "{first}"
        );
        assert!(p.key(key(KeyCode::Char('G'))));
        let last = screen(&mut p, 40, 10).join("\n");
        assert!(last.contains("line 100"), "{last}");
        assert!(!last.contains("line 1 "), "{last}");
        assert!(p.key(key(KeyCode::Char('g'))));
        assert!(screen(&mut p, 40, 10).join("\n").contains("line 1 "));
        assert!(!p.key(key(KeyCode::Esc)));
    }

    #[test]
    fn long_lines_wrap_at_the_width() {
        assert_eq!(wrap("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
    }
}
