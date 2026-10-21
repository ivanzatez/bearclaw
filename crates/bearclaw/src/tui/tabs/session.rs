//! One session up close, inside the Sessions tab: its screen (peek), its brief, and a line to
//! send it a message.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::input::{InputResult, LineInput};
use super::loader::Loader;
use super::pager::wrap;
use crate::cli::render::{clean_text, fit, home_rel};
use crate::core::backend;
use crate::core::brief::{self, Brief};
use crate::core::discovery::Session;

/// How often an open detail re-reads the session's screen.
const PEEK_EVERY: Duration = Duration::from_secs(2);

type PeekFn = fn(&Session) -> Result<String, String>;
type SendFn = fn(&Session, &str) -> Result<(), String>;
type FocusFn = fn(&Session) -> Result<(), String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailOutcome {
    None,
    Close,
    Quit,
    Log(String),
}

pub struct SessionDetail {
    /// The session's stable key — rows re-sort under us, an index would drift.
    pub key: String,
    pub(crate) peek: Loader<String>,
    brief: Option<Brief>,
    brief_loaded: bool,
    input: Option<LineInput>,
    sent: Option<Receiver<Result<String, String>>>,
    peek_fn: PeekFn,
    send_fn: SendFn,
    focus_fn: FocusFn,
}

fn real_peek(s: &Session) -> Result<String, String> {
    backend::peek(s).map_err(|e| e.to_string())
}
fn real_send(s: &Session, text: &str) -> Result<(), String> {
    backend::send(s, text).map_err(|e| e.to_string())
}
fn real_focus(s: &Session) -> Result<(), String> {
    backend::focus(s).map_err(|e| e.to_string())
}

impl SessionDetail {
    pub fn open(key: String) -> Self {
        Self::open_with(key, real_peek, real_send, real_focus)
    }

    /// With the backend calls swapped out — tests must never type into a real terminal.
    pub fn open_with(key: String, peek_fn: PeekFn, send_fn: SendFn, focus_fn: FocusFn) -> Self {
        SessionDetail {
            key,
            peek: Loader::new(),
            brief: None,
            brief_loaded: false,
            input: None,
            sent: None,
            peek_fn,
            send_fn,
            focus_fn,
        }
    }

    fn find<'a>(&self, rows: &'a [Session]) -> Option<&'a Session> {
        rows.iter().find(|r| r.row_key() == self.key)
    }

    fn load_brief(&mut self, s: &Session) {
        self.brief_loaded = true;
        self.brief = s.session_id.as_deref().and_then(|id| {
            let (b, exists) = brief::load(&brief::briefs_dir().ok()?, id).ok()?;
            exists.then_some(b)
        });
    }

    pub fn typing(&self) -> bool {
        self.input.is_some()
    }

    pub fn paste(&mut self, text: &str) {
        if let Some(input) = self.input.as_mut() {
            input.paste(text);
        }
    }

    /// Re-peek on schedule and collect a finished send. Returns a line for the events log.
    pub fn tick(&mut self, rows: &[Session]) -> Option<String> {
        self.peek.poll();
        if let Some(s) = self.find(rows).cloned() {
            if !self.brief_loaded {
                self.load_brief(&s);
            }
            if self.peek.stale(PEEK_EVERY) {
                let f = self.peek_fn;
                self.peek.start(move || f(&s));
            }
        }
        let rx = self.sent.as_ref()?;
        let msg = rx.try_recv().ok()?;
        self.sent = None;
        Some(match msg {
            Ok(m) => m,
            Err(e) => format!("send failed: {e}"),
        })
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, rows: &[Session]) {
        let Some(s) = self.find(rows) else {
            f.render_widget(
                Paragraph::new(" session ended — esc back")
                    .style(Style::default().fg(Color::DarkGray)),
                area,
            );
            return;
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(fit(
                &format!(" {} · {} ", s.headline(), s.status),
                area.width.saturating_sub(2) as usize,
            ))
            .title_bottom(Line::styled(
                " s send · ⏎ focus · r re-read · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let width = inner.width as usize;

        let mut top: Vec<Line> = vec![Line::styled(
            format!(
                " {}{}",
                s.cwd.as_deref().map(home_rel).unwrap_or_default(),
                s.host
                    .as_deref()
                    .map(|h| format!(" · {h}"))
                    .unwrap_or_default()
            ),
            Style::default().fg(Color::Gray),
        )];
        if let Some(b) = &self.brief {
            let bold = Style::default().add_modifier(Modifier::BOLD);
            if !b.summary.trim().is_empty() {
                top.push(Line::styled(" Summary", bold));
                for l in wrap(b.summary.trim(), width.saturating_sub(2))
                    .into_iter()
                    .take(4)
                {
                    top.push(Line::raw(format!("  {l}")));
                }
            }
            let open: Vec<_> = b.todos.iter().filter(|t| !t.done).take(5).collect();
            if !open.is_empty() {
                top.push(Line::styled(" Todo", bold));
                for t in open {
                    top.push(Line::raw(format!(
                        "  ☐ {}",
                        fit(&t.text, width.saturating_sub(5))
                    )));
                }
            }
        }
        let [head, screen, prompt] = Layout::vertical([
            Constraint::Length(top.len() as u16 + 1),
            Constraint::Min(0),
            Constraint::Length(u16::from(self.input.is_some())),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(top), head);

        let lines: Vec<Line> = match (&self.peek.data, &self.peek.error) {
            (_, Some(e)) => vec![Line::styled(
                format!(" {e}"),
                Style::default().fg(Color::Red),
            )],
            (Some(text), None) => {
                // The bottom of the screen is where the session is.
                let all: Vec<&str> = text.trim_end().lines().collect();
                let from = all.len().saturating_sub(screen.height as usize);
                all[from..]
                    .iter()
                    .map(|l| Line::from(Span::raw(fit(&clean_text(l), width))))
                    .collect()
            }
            (None, None) => vec![Line::styled(
                " reading the screen…",
                Style::default().fg(Color::DarkGray),
            )],
        };
        f.render_widget(Paragraph::new(lines), screen);
        if let Some(input) = &self.input {
            input.draw(f, prompt);
        }
    }

    pub fn key(&mut self, k: KeyEvent, rows: &[Session]) -> DetailOutcome {
        if let Some(input) = self.input.as_mut() {
            match input.key(k) {
                InputResult::Editing => {}
                InputResult::Cancel => self.input = None,
                InputResult::Quit => return DetailOutcome::Quit,
                InputResult::Submit(text) => {
                    self.input = None;
                    if text.trim().is_empty() {
                        return DetailOutcome::None;
                    }
                    let Some(s) = self.find(rows).cloned() else {
                        return DetailOutcome::Log("not sent: the session ended".into());
                    };
                    let (tx, rx) = channel();
                    let send = self.send_fn;
                    std::thread::spawn(move || {
                        let label = s.label();
                        let _ = tx.send(send(&s, &text).map(|()| format!("sent to {label}")));
                    });
                    self.sent = Some(rx);
                }
            }
            return DetailOutcome::None;
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) {
            return if k.code == KeyCode::Char('c') {
                DetailOutcome::Quit
            } else {
                DetailOutcome::None
            };
        }
        match k.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => DetailOutcome::Close,
            KeyCode::Char('q') => DetailOutcome::Quit,
            KeyCode::Char('s') => {
                self.input = Some(LineInput::new("send"));
                DetailOutcome::None
            }
            KeyCode::Char('r') => {
                self.brief_loaded = false;
                self.peek = Loader::new();
                DetailOutcome::None
            }
            KeyCode::Enter | KeyCode::Char('o') => match self.find(rows) {
                Some(s) => match (self.focus_fn)(s) {
                    Ok(()) => DetailOutcome::None,
                    Err(e) => DetailOutcome::Log(format!("focus failed: {e}")),
                },
                None => DetailOutcome::None,
            },
            _ => DetailOutcome::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    static SENT: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn no_peek(_: &Session) -> Result<String, String> {
        Err("fake: peeks are off in tests".into())
    }
    fn record_send(_: &Session, text: &str) -> Result<(), String> {
        SENT.lock().unwrap().push(text.to_string());
        Ok(())
    }
    fn no_focus(_: &Session) -> Result<(), String> {
        Ok(())
    }

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn rows() -> Vec<Session> {
        vec![Session {
            name: Some("docs-refresh".into()),
            session_id: Some("id-docs".into()),
            cwd: Some("/tmp/wt/docs".into()),
            status: "idle".into(),
            ..Default::default()
        }]
    }

    fn screen(d: &mut SessionDetail, rows: &[Session]) -> String {
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| d.draw(f, f.area(), rows)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..30)
            .map(|y| (0..100).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
            .collect()
    }

    /// Tick until `done` holds (background work settles within the deadline).
    fn settle(
        d: &mut SessionDetail,
        rows: &[Session],
        mut done: impl FnMut(&SessionDetail, Option<String>) -> bool,
    ) {
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            let msg = d.tick(rows);
            if done(d, msg) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("background work did not settle");
    }

    #[test]
    fn detail_pins_its_session_and_sends_through_input() {
        let rows = rows();
        let key = rows[0].row_key();
        let mut d = SessionDetail::open_with(key, no_peek, record_send, no_focus);
        settle(&mut d, &rows, |d, _| !d_peek_loading(d));
        let s = screen(&mut d, &rows);
        assert!(
            s.contains("docs-refresh") && s.contains("/tmp/wt/docs"),
            "{s}"
        );
        assert!(s.contains("peeks are off in tests"), "{s}");
        assert!(screen(&mut d, &[]).contains("session ended"));

        assert_eq!(d.key(k(KeyCode::Char('s')), &rows), DetailOutcome::None);
        assert!(d.typing());
        assert_eq!(d.key(k(KeyCode::Char('q')), &rows), DetailOutcome::None);
        assert!(d.typing());
        d.key(k(KeyCode::Esc), &rows);
        assert!(!d.typing());

        d.key(k(KeyCode::Char('s')), &rows);
        for c in "hi there".chars() {
            d.key(k(KeyCode::Char(c)), &rows);
        }
        assert_eq!(d.key(k(KeyCode::Enter), &rows), DetailOutcome::None);
        let mut log = None;
        settle(&mut d, &rows, |_, m| {
            log = m;
            log.is_some()
        });
        assert!(log.unwrap().contains("sent"));
        assert!(SENT.lock().unwrap().contains(&"hi there".to_string()));
        assert_eq!(d.key(k(KeyCode::Esc), &rows), DetailOutcome::Close);
    }

    fn d_peek_loading(d: &SessionDetail) -> bool {
        d.peek.loading || d.peek.error.is_none() && d.peek.data.is_none()
    }
}
