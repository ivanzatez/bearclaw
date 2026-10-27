//! The QA tab: QA sessions, where a presenter session talks the user through someone's work by
//! voice and they interrupt it with questions. A terminal has no microphone or speaker, so ⏎ opens
//! one in the web app; `n` opens the web app's QA list to start a new one; `s` shows the presenter.

use std::collections::HashMap;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::loader::Loader;
use super::{Outcome, View, common_key};
use crate::cli::render::{ago, column, fit};
use crate::core::qa::{self, Qa};
use crate::core::tools::open_url;

const TTL: Duration = Duration::from_secs(30);

pub struct QaView {
    pub(crate) load: Loader<Vec<Qa>>,
    sel: usize,
    list: ListState,
    /// Live sessions' ids by tmux session: which presenters still run.
    tmux: HashMap<String, String>,
    /// How a link opens — swapped out in tests, which must not launch a browser.
    pub(crate) opener: fn(&str) -> Result<(), String>,
}

impl Default for QaView {
    fn default() -> Self {
        Self::new()
    }
}

impl QaView {
    pub fn new() -> Self {
        QaView {
            load: Loader::new(),
            sel: 0,
            list: ListState::default(),
            tmux: HashMap::new(),
            opener: open_url,
        }
    }

    fn fetch(&mut self) {
        self.load.start(|| Ok(qa::list()));
    }

    fn sessions_list(&self) -> &[Qa] {
        self.load.data.as_deref().unwrap_or(&[])
    }

    fn selected(&self) -> Option<&Qa> {
        self.sessions_list().get(self.sel)
    }

    fn row(&self, q: &Qa, width: usize) -> Line<'static> {
        let live = self.tmux.contains_key(&q.presenter);
        let (mark, mark_color) = if live {
            ("●", Color::Green)
        } else {
            ("○", Color::DarkGray)
        };
        let source = q.source.clone().unwrap_or_else(|| "topic".into());
        let source_w = if width >= 90 { 32 } else { 0 };
        let title_w = width.saturating_sub(3 + source_w + 4 + 6);
        let mut spans = vec![
            Span::styled(format!(" {mark} "), Style::default().fg(mark_color)),
            Span::raw(column(&q.title, title_w)),
        ];
        if source_w > 0 {
            spans.push(Span::styled(
                format!(" {}", column(&source, source_w - 1)),
                Style::default().fg(Color::Gray),
            ));
        }
        spans.push(Span::styled(
            format!(" {} ", column(&q.lang, 2)),
            Style::default().fg(Color::Cyan),
        ));
        spans.push(Span::styled(
            column(&ago(q.created_at), 4),
            Style::default().fg(Color::DarkGray),
        ));
        Line::from(spans)
    }

    fn open(&self, id: Option<&str>) -> Outcome {
        match (self.opener)(&qa::url(id)) {
            Err(e) => Outcome::Log(e),
            Ok(()) => Outcome::None,
        }
    }
}

impl View for QaView {
    fn on_show(&mut self) {
        if self.load.stale(TTL) {
            self.fetch();
        }
    }

    fn refresh(&mut self) {
        self.fetch();
    }

    fn tick(&mut self) {
        if self.load.poll() {
            self.sel = self.sel.min(self.sessions_list().len().saturating_sub(1));
        }
    }

    fn sessions(&mut self, rows: &[crate::core::discovery::Session]) {
        self.tmux = rows
            .iter()
            .filter_map(|r| Some((r.tmux_session.clone()?, r.session_id.clone()?)))
            .collect();
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        let mut title = format!(" QA · {} ", self.sessions_list().len());
        if self.load.loading {
            title.push_str("· loading… ");
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(fit(&title, area.width.saturating_sub(2) as usize))
            .title_bottom(Line::styled(
                " ⏎ open in the browser · n new QA · s the presenter · r refresh · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if self.load.data.is_none() {
            let msg = if self.load.loading {
                "looking for QA sessions…"
            } else {
                "r to look for QA sessions"
            };
            f.render_widget(Paragraph::new(format!(" {msg}")), inner);
            return;
        }
        if self.sessions_list().is_empty() {
            f.render_widget(
                Paragraph::new(
                    " No QA sessions yet. n opens the web app: pick whose work to present, and a new session talks you through it by voice.",
                )
                .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        let width = inner.width as usize;
        let items: Vec<ListItem> = self
            .sessions_list()
            .iter()
            .map(|q| ListItem::new(self.row(q, width)))
            .collect();
        self.list.select(Some(self.sel));
        let list = List::new(items).highlight_style(Style::default().bg(Color::DarkGray));
        f.render_stateful_widget(list, inner, &mut self.list);
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if let Some(o) = common_key(k) {
            return o;
        }
        match k.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.sel = (self.sel + 1).min(self.sessions_list().len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('n') => return self.open(None),
            KeyCode::Enter | KeyCode::Char('o') => {
                if let Some(id) = self.selected().map(|q| q.id.clone()) {
                    return self.open(Some(&id));
                }
            }
            KeyCode::Char('s') => {
                return match self
                    .selected()
                    .and_then(|q| self.tmux.get(&q.presenter).cloned())
                {
                    Some(id) => Outcome::Session(id),
                    None => Outcome::Log("that QA session's presenter is not running".into()),
                };
            }
            _ => {}
        }
        Outcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen(v: &mut QaView, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    static OPENED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn record(url: &str) -> Result<(), String> {
        OPENED.lock().unwrap().push(url.to_string());
        Ok(())
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn lists_qa_sessions_and_opens_them_in_the_web_app() {
        let mut v = QaView::new();
        v.opener = record;
        v.load.data = Some(vec![
            Qa {
                id: "subagents-ab12".into(),
                title: "Subagents".into(),
                presenter: "qa-subagents-ab12".into(),
                source: Some("agent-orchestration-subagent".into()),
                lang: "hr".into(),
                created_at: None,
            },
            Qa {
                id: "old-0000".into(),
                title: "Older".into(),
                presenter: "qa-old-0000".into(),
                ..Default::default()
            },
        ]);
        v.tmux
            .insert("qa-subagents-ab12".into(), "presenter-id".into());
        let s = screen(&mut v, 120, 8);
        assert!(
            s.contains("● Subagents") && s.contains("agent-orchestration-subagent"),
            "{s}"
        );
        assert!(s.contains("○ Older") && s.contains("topic"), "{s}");

        assert_eq!(v.key(key(KeyCode::Enter)), Outcome::None);
        assert_eq!(v.key(key(KeyCode::Char('n'))), Outcome::None);
        let opened = OPENED.lock().unwrap().clone();
        assert!(
            opened.iter().any(|u| u.ends_with("/#/qa/subagents-ab12")),
            "{opened:?}"
        );
        assert!(opened.iter().any(|u| u.ends_with("/#/qa")), "{opened:?}");
        assert_eq!(
            v.key(key(KeyCode::Char('s'))),
            Outcome::Session("presenter-id".into())
        );
        v.key(key(KeyCode::Char('j')));
        assert!(matches!(v.key(key(KeyCode::Char('s'))), Outcome::Log(_)));
        assert_eq!(v.key(key(KeyCode::Esc)), Outcome::Home);
    }

    #[test]
    fn says_how_to_start_one_when_there_are_none() {
        let mut v = QaView::new();
        v.load.data = Some(Vec::new());
        assert!(screen(&mut v, 100, 6).contains("No QA sessions yet"));
    }
}
