//! The Videos tab: the narrated videos sessions made about their work. A terminal plays neither
//! sound nor animation, so ⏎ opens one in the web app, where you pause it and ask its session by
//! voice; `s` shows that session here.

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
use crate::core::title::display_title;
use crate::core::tools::open_url;
use crate::core::videos::{self, Video};

const TTL: Duration = Duration::from_secs(30);

pub struct VideosView {
    pub(crate) load: Loader<Vec<Video>>,
    sel: usize,
    list: ListState,
    /// Live sessions' titles by session id: who answers each video's questions.
    titles: HashMap<String, String>,
    /// How a video opens — swapped out in tests, which must not launch a browser.
    pub(crate) opener: fn(&str) -> Result<(), String>,
}

impl Default for VideosView {
    fn default() -> Self {
        Self::new()
    }
}

impl VideosView {
    pub fn new() -> Self {
        VideosView {
            load: Loader::new(),
            sel: 0,
            list: ListState::default(),
            titles: HashMap::new(),
            opener: open_url,
        }
    }

    fn fetch(&mut self) {
        self.load.start(|| Ok(videos::list()));
    }

    fn videos(&self) -> &[Video] {
        self.load.data.as_deref().unwrap_or(&[])
    }

    fn selected(&self) -> Option<&Video> {
        self.videos().get(self.sel)
    }

    /// Who answers its questions: the session's title while it runs.
    fn answerer(&self, v: &Video) -> (String, Color) {
        match &v.session {
            Some(id) => match self.titles.get(id) {
                Some(t) => (t.clone(), Color::Green),
                None => ("its session is not running".into(), Color::DarkGray),
            },
            None => ("no session".into(), Color::DarkGray),
        }
    }

    fn row(&self, v: &Video, width: usize) -> Line<'static> {
        let len = format!(" {} ", column(&videos::clock(v.duration), 5));
        let scenes = format!("{:>2} scenes ", v.scenes);
        let age = column(
            &ago(v.updated.and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_millis() as i64)
            })),
            4,
        );
        let (who, who_color) = self.answerer(v);
        let who_w = if width >= 90 { 30 } else { 0 };
        let title_w = width.saturating_sub(7 + 10 + who_w + 6);
        let mut spans = vec![
            Span::styled(len, Style::default().fg(Color::Cyan)),
            Span::raw(column(&v.title, title_w)),
            Span::styled(format!(" {scenes}"), Style::default().fg(Color::Gray)),
        ];
        if who_w > 0 {
            spans.push(Span::styled(
                format!("{} ", column(&who, who_w - 1)),
                Style::default().fg(who_color),
            ));
        }
        spans.push(Span::styled(
            format!(" {age}"),
            Style::default().fg(Color::DarkGray),
        ));
        Line::from(spans)
    }

    fn open(&self) -> Outcome {
        match self.selected().map(|v| (self.opener)(&videos::url(&v.id))) {
            Some(Err(e)) => Outcome::Log(e),
            _ => Outcome::None,
        }
    }
}

impl View for VideosView {
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
            self.sel = self.sel.min(self.videos().len().saturating_sub(1));
        }
    }

    fn sessions(&mut self, rows: &[crate::core::discovery::Session]) {
        self.titles = rows
            .iter()
            .filter_map(|r| r.session_id.clone().map(|id| (id, display_title(r))))
            .collect();
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        let mut title = format!(" Videos · {} ", self.videos().len());
        if self.load.loading {
            title.push_str("· loading… ");
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(fit(&title, area.width.saturating_sub(2) as usize))
            .title_bottom(Line::styled(
                " ⏎ watch and ask in the browser · s its session · r refresh · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if self.load.data.is_none() {
            let msg = if self.load.loading {
                "looking for videos…"
            } else {
                "r to look for videos"
            };
            f.render_widget(Paragraph::new(format!(" {msg}")), inner);
            return;
        }
        if self.videos().is_empty() {
            let msg = format!(
                " No videos yet. A session makes one in {} (one folder each).",
                videos::dir().display()
            );
            f.render_widget(Paragraph::new(msg).wrap(Wrap { trim: false }), inner);
            return;
        }
        let width = inner.width as usize;
        let items: Vec<ListItem> = self
            .videos()
            .iter()
            .map(|v| ListItem::new(self.row(v, width)))
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
                self.sel = (self.sel + 1).min(self.videos().len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Enter | KeyCode::Char('o') => return self.open(),
            KeyCode::Char('s') => {
                return match self.selected().and_then(|v| v.session.clone()) {
                    Some(id) if self.titles.contains_key(&id) => Outcome::Session(id),
                    Some(_) => {
                        Outcome::Log("the session that made this video is not running".into())
                    }
                    None => Outcome::Log("this video names no session".into()),
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

    fn screen(v: &mut VideosView, w: u16, h: u16) -> String {
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
    fn lists_videos_and_opens_one_in_the_web_app() {
        let mut v = VideosView::new();
        v.opener = record;
        v.load.data = Some(vec![
            Video {
                id: "subagents".into(),
                title: "Subagents in the AI Agent".into(),
                session: Some("abc".into()),
                duration: 147.5,
                scenes: 9,
                updated: None,
            },
            Video {
                id: "old".into(),
                title: "Older one".into(),
                session: Some("gone".into()),
                duration: 30.0,
                scenes: 3,
                updated: None,
            },
        ]);
        v.titles
            .insert("abc".into(), "agent-orchestration-subagent".into());
        let s = screen(&mut v, 120, 8);
        assert!(
            s.contains("2:27")
                && s.contains("Subagents in the AI Agent")
                && s.contains(" 9 scenes"),
            "{s}"
        );
        assert!(
            s.contains("agent-orchestration-subagent") && s.contains("its session is not running"),
            "{s}"
        );

        assert_eq!(v.key(key(KeyCode::Enter)), Outcome::None);
        assert!(
            OPENED
                .lock()
                .unwrap()
                .iter()
                .any(|u| u.ends_with("/#/videos/subagents"))
        );
        assert_eq!(
            v.key(key(KeyCode::Char('s'))),
            Outcome::Session("abc".into())
        );
        v.key(key(KeyCode::Char('j')));
        assert!(matches!(v.key(key(KeyCode::Char('s'))), Outcome::Log(_)));
        assert_eq!(v.key(key(KeyCode::Esc)), Outcome::Home);
    }

    #[test]
    fn says_where_videos_come_from_when_there_are_none() {
        let mut v = VideosView::new();
        v.load.data = Some(Vec::new());
        assert!(screen(&mut v, 100, 6).contains("No videos yet"));
    }
}
