//! The Tasks tab: your open Productive tasks.

use std::time::Duration;

use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::loader::Loader;
use super::pager::Pager;
use super::{Outcome, View, common_key};
use crate::cli::render::{column, fit};
use crate::core::productive::{self, Task};
use crate::core::tools::open_url;

const TTL: Duration = Duration::from_secs(120);

pub struct TasksView {
    pub(crate) load: Loader<Vec<Task>>,
    sel: usize,
    list: ListState,
    pager: Option<Pager>,
}

impl Default for TasksView {
    fn default() -> Self {
        Self::new()
    }
}

/// `02.01.` and how it reads against today: overdue red, due today yellow.
fn due_cell(due: Option<&str>, today: NaiveDate) -> (String, Color) {
    let Some(d) = due.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) else {
        return (String::new(), Color::DarkGray);
    };
    let color = match d.cmp(&today) {
        std::cmp::Ordering::Less => Color::Red,
        std::cmp::Ordering::Equal => Color::Yellow,
        std::cmp::Ordering::Greater => Color::Gray,
    };
    (d.format("%d.%m.").to_string(), color)
}

impl TasksView {
    pub fn new() -> Self {
        TasksView {
            load: Loader::new(),
            sel: 0,
            list: ListState::default(),
            pager: None,
        }
    }

    fn fetch(&mut self) {
        self.load
            .start(|| productive::Settings::load().and_then(|s| productive::my_open_tasks(&s)));
    }

    fn tasks(&self) -> &[Task] {
        self.load.data.as_deref().unwrap_or(&[])
    }

    fn row(t: &Task, width: usize, today: NaiveDate) -> Line<'static> {
        let num = format!(" {} ", column(&format!("#{}", t.number), 6));
        let (due, due_color) = due_cell(t.due_date.as_deref(), today);
        let due = format!(" {} ", column(&due, 6));
        // Drop the side columns before the title gets unreadable.
        let (project_w, status_w) = match width {
            w if w >= 90 => (20, 14),
            w if w >= 64 => (0, 14),
            _ => (0, 0),
        };
        let title_w = width.saturating_sub(6 + 2 + 6 + 2 + project_w + status_w + 2);
        let mut spans = vec![
            Span::styled(num, Style::default().fg(Color::Cyan)),
            Span::raw(column(&t.title, title_w)),
        ];
        if project_w > 0 {
            spans.push(Span::styled(
                format!(" {}", column(&t.project, project_w - 1)),
                Style::default().fg(Color::Gray),
            ));
        }
        if status_w > 0 {
            spans.push(Span::styled(
                format!(" {}", column(&t.status, status_w - 1)),
                Style::default().fg(Color::Magenta),
            ));
        }
        spans.push(Span::styled(due, Style::default().fg(due_color)));
        Line::from(spans)
    }

    fn open(&mut self) {
        let Some(t) = self.tasks().get(self.sel) else {
            return;
        };
        let text = format!(
            "{}\n\n#{} · {} · {}\nDue: {}\n{}\n\n{}",
            t.title,
            t.number,
            t.project,
            t.status,
            t.due_date.as_deref().unwrap_or("—"),
            t.url,
            if t.description.is_empty() {
                "(no description)"
            } else {
                &t.description
            }
        );
        self.pager = Some(Pager::new(format!("#{} {}", t.number, t.title), text));
    }
}

impl View for TasksView {
    fn on_show(&mut self) {
        if self.load.stale(TTL) {
            self.fetch();
        }
    }

    fn tick(&mut self) {
        if self.load.poll() {
            self.sel = self.sel.min(self.tasks().len().saturating_sub(1));
        }
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        if let Some(p) = self.pager.as_mut() {
            p.draw(f, area);
            return;
        }
        let mut title = format!(" Tasks · {} open ", self.tasks().len());
        if self.load.data.is_some()
            && let Some(e) = &self.load.error
        {
            title.push_str(&format!("· {e} "));
        }
        if self.load.loading {
            title.push_str("· loading… ");
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(fit(&title, area.width.saturating_sub(2) as usize))
            .title_bottom(Line::styled(
                " ⏎ read · o open · r refresh · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if self.load.data.is_none() {
            let msg = match (&self.load.error, self.load.loading) {
                (Some(e), _) => e.clone(),
                (None, true) => "asking Productive…".into(),
                (None, false) => "r to fetch your tasks".into(),
            };
            f.render_widget(
                Paragraph::new(format!(" {msg}")).wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        if self.tasks().is_empty() {
            f.render_widget(Paragraph::new(" no open tasks"), inner);
            return;
        }
        let today = chrono::Local::now().date_naive();
        let width = inner.width as usize;
        let items: Vec<ListItem> = self
            .tasks()
            .iter()
            .map(|t| ListItem::new(Self::row(t, width, today)))
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
                self.sel = (self.sel + 1).min(self.tasks().len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('r') => self.fetch(),
            KeyCode::Enter => self.open(),
            KeyCode::Char('o') => {
                if let Some(t) = self.tasks().get(self.sel)
                    && let Err(e) = open_url(&t.url)
                {
                    return Outcome::Log(e);
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen(v: &mut TasksView, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn tasks_tab_shows_errors_inline() {
        let mut v = TasksView::new();
        v.load.error = Some("set productive.organizationId: bearclaw config set".into());
        assert!(screen(&mut v, 80, 10).contains("set productive.organizationId"));
        let j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(v.key(j), Outcome::None);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(v.key(enter), Outcome::None);
    }

    #[test]
    fn enter_opens_the_task_and_esc_closes_it() {
        let mut v = TasksView::new();
        v.load.data = Some(vec![Task {
            id: "7".into(),
            number: "12".into(),
            title: "Fix login".into(),
            project: "Website".into(),
            status: "In progress".into(),
            due_date: Some("2000-01-02".into()),
            description: "the details".into(),
            url: "https://app.example/tasks/7".into(),
            ..Default::default()
        }]);
        let list = screen(&mut v, 100, 10);
        assert!(
            list.contains("#12") && list.contains("Fix login") && list.contains("Website"),
            "{list}"
        );
        assert!(list.contains("02.01."), "{list}");
        v.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let open = screen(&mut v, 100, 20);
        assert!(
            open.contains("the details") && open.contains("In progress"),
            "{open}"
        );
        assert_eq!(
            v.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Outcome::None
        );
        assert!(screen(&mut v, 100, 10).contains("#12"));
        assert_eq!(
            v.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Outcome::Home
        );
    }
}
