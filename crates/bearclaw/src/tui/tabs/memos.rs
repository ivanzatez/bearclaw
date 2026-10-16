//! The Memos tab: voice memos' transcripts, read from the local web server.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use serde_json::Value;

use super::loader::Loader;
use super::pager::Pager;
use super::{Outcome, View, common_key};
use crate::cli::render::{column, fit, width_of};
use crate::core::webapi;

const TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
pub struct Memo {
    pub id: String,
    pub title: String,
    /// ISO 8601 with offset.
    pub created: Option<String>,
    pub duration_sec: Option<f64>,
    /// `done` or `partial`.
    pub status: String,
    pub words: u64,
    pub preview: String,
}

/// `GET /api/memos` → its memos, newest first as the server sends them.
pub fn parse_memos(v: &Value) -> Vec<Memo> {
    v["memos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let s = |k: &str| m[k].as_str().unwrap_or("").to_string();
            Memo {
                id: s("id"),
                title: s("title"),
                created: m["created"].as_str().map(str::to_string),
                duration_sec: m["durationSec"].as_f64(),
                status: s("status"),
                words: m["words"].as_u64().unwrap_or(0),
                preview: s("preview"),
            }
        })
        .collect()
}

pub struct MemosView {
    pub(crate) load: Loader<Vec<Memo>>,
    /// The opened memo: (title, transcript).
    detail: Loader<(String, String)>,
    sel: usize,
    list: ListState,
    pager: Option<Pager>,
}

impl Default for MemosView {
    fn default() -> Self {
        Self::new()
    }
}

impl MemosView {
    pub fn new() -> Self {
        MemosView {
            load: Loader::new(),
            detail: Loader::new(),
            sel: 0,
            list: ListState::default(),
            pager: None,
        }
    }

    fn fetch(&mut self) {
        self.load
            .start(|| webapi::get_json("/api/memos").map(|v| parse_memos(&v)));
    }

    fn memos(&self) -> &[Memo] {
        self.load.data.as_deref().unwrap_or(&[])
    }

    fn row(m: &Memo, width: usize) -> Line<'static> {
        let when = m
            .created
            .as_deref()
            .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
            .map(|d| d.format("%d.%m. %H:%M").to_string())
            .unwrap_or_default();
        let dur = m
            .duration_sec
            .map(|d| format!("{}:{:02}", d as u64 / 60, d as u64 % 60))
            .unwrap_or_default();
        let meta = format!(
            " {} {} {:>5}w ",
            column(&when, 12),
            column(&dur, 6),
            m.words
        );
        let mark = if m.status == "partial" { " ◐" } else { "  " };
        let title_w = width.saturating_sub(1 + width_of(&meta) + 2);
        Line::from(vec![
            Span::raw(format!(" {}", column(&m.title, title_w))),
            Span::styled(meta, Style::default().fg(Color::DarkGray)),
            Span::styled(mark, Style::default().fg(Color::Yellow)),
        ])
    }
}

impl View for MemosView {
    fn on_show(&mut self) {
        if self.load.stale(TTL) {
            self.fetch();
        }
    }

    fn tick(&mut self) {
        if self.load.poll() {
            self.sel = self.sel.min(self.memos().len().saturating_sub(1));
        }
        if self.detail.poll() {
            self.pager = Some(match (&self.detail.data, &self.detail.error) {
                (_, Some(e)) => Pager::new("Memo".into(), e.clone()),
                (Some((title, text)), None) => Pager::new(title.clone(), text.clone()),
                (None, None) => return,
            });
            self.detail.data = None;
            self.detail.error = None;
        }
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        if let Some(p) = self.pager.as_mut() {
            p.draw(f, area);
            return;
        }
        let mut title = format!(" Memos · {} ", self.memos().len());
        if self.load.loading || self.detail.loading {
            title.push_str("· loading… ");
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(fit(&title, area.width.saturating_sub(2) as usize))
            .title_bottom(Line::styled(
                " ⏎ read · r refresh · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = match (&self.load.error, self.load.loading, self.memos().is_empty()) {
            (Some(e), _, true) => Some(e.clone()),
            (None, true, true) => Some("asking the web server…".into()),
            (None, false, true) => Some("no memos yet — record them in the web UI".into()),
            _ => None,
        };
        if let Some(msg) = msg {
            f.render_widget(
                Paragraph::new(format!(" {msg}")).wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        let width = inner.width as usize;
        let items: Vec<ListItem> = self
            .memos()
            .iter()
            .map(|m| ListItem::new(Self::row(m, width)))
            .collect();
        self.list.select(Some(self.sel));
        let list = List::new(items).highlight_style(Style::default().bg(Color::DarkGray));
        f.render_stateful_widget(list, inner, &mut self.list);
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if let Some(p) = self.pager.as_mut() {
            if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
                return Outcome::Quit;
            }
            if !p.key(k) {
                self.pager = None;
            }
            return Outcome::None;
        }
        if let Some(o) = common_key(k) {
            return o;
        }
        match k.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.sel = (self.sel + 1).min(self.memos().len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('r') => self.fetch(),
            KeyCode::Enter => {
                if let Some(m) = self.memos().get(self.sel) {
                    let (id, title) = (m.id.clone(), m.title.clone());
                    self.detail.start(move || {
                        let v = webapi::get_json(&format!("/api/memos/{id}"))?;
                        Ok((title, v["transcript"].as_str().unwrap_or("").to_string()))
                    });
                }
            }
            _ => {}
        }
        Outcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    #[test]
    fn memos_parse_in_order() {
        let v = json!({"dir": "/x", "memos": [
            {"id":"b","title":"Standup","created":"2026-10-16T09:30:00+02:00","durationSec":95.0,
             "recording":null,"status":"done","words":240,"preview":"we talked"},
            {"id":"a","title":"Idea","created":null,"durationSec":null,
             "recording":null,"status":"partial","words":3,"preview":"hm"}
        ]});
        let m = parse_memos(&v);
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].id.as_str(), m[0].title.as_str(), m[0].words),
            ("b", "Standup", 240)
        );
        assert_eq!(m[0].duration_sec, Some(95.0));
        assert_eq!(m[1].created, None);
        assert_eq!(m[1].status, "partial");
    }

    #[test]
    fn a_dead_server_is_explained() {
        let mut v = MemosView::new();
        v.load.error = Some(
            "web server not running at http://127.0.0.1:7777 — start it: bearclaw web serve".into(),
        );
        let mut t = Terminal::new(TestBackend::new(100, 8)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        let s: String = (0..8)
            .map(|y| (0..100).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect();
        assert!(s.contains("bearclaw web serve"), "{s}");
    }
}
