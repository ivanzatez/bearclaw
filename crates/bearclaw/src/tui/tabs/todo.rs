//! The Todo tab: one-line work items — yours, and the ones Claude sessions keep current about
//! themselves — each with its session and Productive task when it has them.

use std::path::PathBuf;
use std::time::SystemTime;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use super::input::{InputResult, LineInput};
use super::{Outcome, View, common_key};
use crate::cli::render::{column, fit, width_of};
use crate::core::todo::{self, Item, Status, Todo};
use crate::core::tools::open_url;

/// Done items stay listed (at the bottom) this long.
const DONE_FOR_MS: i64 = 3 * 24 * 60 * 60 * 1000;

pub struct TodoView {
    path: PathBuf,
    todo: Todo,
    error: Option<String>,
    /// The file's mtime when last read: sessions write it, so it is re-read when it moves.
    seen: Option<SystemTime>,
    sel: usize,
    list: ListState,
    input: Option<LineInput>,
    confirm_delete: Option<u64>,
    /// How a task link opens — swapped out in tests, which must not launch a browser.
    pub(crate) opener: fn(&str) -> Result<(), String>,
}

impl Default for TodoView {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoView {
    pub fn new() -> Self {
        Self::at(todo::path())
    }

    pub fn at(path: PathBuf) -> Self {
        TodoView {
            path,
            todo: Todo::default(),
            error: None,
            seen: None,
            sel: 0,
            list: ListState::default(),
            input: None,
            confirm_delete: None,
            opener: open_url,
        }
    }

    fn mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok()
    }

    fn reload(&mut self) {
        self.seen = self.mtime();
        match todo::load(&self.path) {
            Ok(t) => {
                self.todo = t;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        self.sel = self.sel.min(self.order().len().saturating_sub(1));
    }

    /// What is listed, in order: in progress, then open, then recently done — newest first in
    /// each.
    fn order(&self) -> Vec<usize> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut out = Vec::new();
        for status in [Status::Doing, Status::Open, Status::Done] {
            let mut idx: Vec<usize> = (0..self.todo.items.len())
                .filter(|&i| {
                    let it = &self.todo.items[i];
                    it.status == status
                        && (status != Status::Done || now - it.updated_ms < DONE_FOR_MS)
                })
                .collect();
            idx.sort_by_key(|&i| std::cmp::Reverse(self.todo.items[i].updated_ms));
            out.extend(idx);
        }
        out
    }

    fn selected(&self) -> Option<&Item> {
        self.order().get(self.sel).map(|&i| &self.todo.items[i])
    }

    /// Change the file under its lock, then show what is there now.
    fn change(&mut self, f: impl FnOnce(&mut Todo) -> crate::error::Result<()>) -> Outcome {
        let result = todo::update(&self.path, f);
        self.reload();
        match result {
            Ok(()) => Outcome::None,
            Err(e) => Outcome::Log(format!("todo: {e}")),
        }
    }

    fn row(it: &Item, width: usize) -> Line<'static> {
        let (mark, color) = match it.status {
            Status::Doing => ("●", Color::Cyan),
            Status::Open => ("○", Color::Yellow),
            Status::Done => ("✓", Color::DarkGray),
        };
        let who = it
            .session_label
            .as_deref()
            .map(|l| format!("  ⧉ {}", fit(l, 24)))
            .unwrap_or_default();
        let task = if it.task_url.is_some() {
            "  ↗ task"
        } else {
            ""
        };
        let text_w = width.saturating_sub(3 + width_of(&who) + width_of(task) + 1);
        let text_style = if it.status == Status::Done {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };
        Line::from(vec![
            Span::styled(format!(" {mark} "), Style::default().fg(color)),
            Span::styled(column(&it.text, text_w), text_style),
            Span::styled(who, Style::default().fg(Color::Gray)),
            Span::styled(task.to_string(), Style::default().fg(Color::Blue)),
        ])
    }
}

impl View for TodoView {
    fn on_show(&mut self) {
        self.reload();
    }

    fn tick(&mut self) {
        if self.mtime() != self.seen {
            self.reload();
        }
    }

    fn refresh(&mut self) {
        self.reload();
    }

    fn typing(&self) -> bool {
        self.input.is_some() || self.confirm_delete.is_some()
    }

    fn paste(&mut self, text: &str) {
        if let Some(input) = self.input.as_mut() {
            input.paste(text);
        }
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        let doing = self
            .todo
            .items
            .iter()
            .filter(|i| i.status == Status::Doing)
            .count();
        let open = self
            .todo
            .items
            .iter()
            .filter(|i| i.status == Status::Open)
            .count();
        let mut title = format!(" Todo · {doing} doing · {open} open ");
        if let Some(e) = &self.error {
            title.push_str(&format!("· ✕ {e} "));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(fit(&title, area.width.saturating_sub(2) as usize))
            .title_bottom(Line::styled(
                " ⏎ open task/session · n new · x done · c start session · g go · d delete · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let asking = self.input.is_some() || self.confirm_delete.is_some();
        let [body, prompt] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(u16::from(asking))])
                .areas(inner);
        if let Some(input) = &self.input {
            input.draw(f, prompt);
        }
        if let Some(id) = self.confirm_delete {
            f.render_widget(
                Paragraph::new(Line::styled(
                    format!(" delete #{id}? y/n"),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                prompt,
            );
        }
        let order = self.order();
        if order.is_empty() {
            f.render_widget(
                Paragraph::new(" nothing to do yet — n to add, or let a session add its own"),
                body,
            );
            return;
        }
        let width = body.width as usize;
        let items: Vec<ListItem> = order
            .iter()
            .map(|&i| ListItem::new(Self::row(&self.todo.items[i], width)))
            .collect();
        self.list.select(Some(self.sel));
        let list = List::new(items).highlight_style(Style::default().bg(Color::DarkGray));
        f.render_stateful_widget(list, body, &mut self.list);
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if let Some(input) = self.input.as_mut() {
            return match input.key(k) {
                InputResult::Editing => Outcome::None,
                InputResult::Quit => Outcome::Quit,
                InputResult::Cancel => {
                    self.input = None;
                    Outcome::None
                }
                InputResult::Submit(text) => {
                    self.input = None;
                    if text.trim().is_empty() {
                        return Outcome::None;
                    }
                    self.change(|t| {
                        t.add(&text, None, None);
                        Ok(())
                    })
                }
            };
        }
        if let Some(id) = self.confirm_delete.take() {
            if k.code == KeyCode::Char('y') {
                return self.change(|t| t.remove(id));
            }
            return Outcome::None;
        }
        if let Some(o) = common_key(k) {
            return o;
        }
        let selected = self.selected().cloned();
        match k.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.sel = (self.sel + 1).min(self.order().len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('n') => self.input = Some(LineInput::new("new todo")),
            KeyCode::Char('d') => self.confirm_delete = selected.map(|i| i.id),
            KeyCode::Char('x') => {
                if let Some(it) = selected {
                    return self.change(|t| {
                        if it.status == Status::Done {
                            t.reopen(it.id)
                        } else {
                            t.done(it.id)
                        }
                    });
                }
            }
            KeyCode::Enter => match selected {
                Some(Item {
                    task_url: Some(url),
                    ..
                }) => {
                    if let Err(e) = (self.opener)(&url) {
                        return Outcome::Log(e);
                    }
                }
                Some(Item {
                    session: Some(s), ..
                }) => return Outcome::Session(s),
                _ => {}
            },
            KeyCode::Char('g') => {
                if let Some(s) = selected.and_then(|i| i.session) {
                    return Outcome::Session(s);
                }
            }
            KeyCode::Char('c') => {
                if let Some(it) = selected {
                    let task = it
                        .task_url
                        .as_deref()
                        .map(|u| format!(" — {u}"))
                        .unwrap_or_default();
                    return Outcome::Spawn(format!(
                        "You're picking up todo item #{}: {}{task}. Run `bearclaw todo claim {}` first.",
                        it.id, it.text, it.id
                    ));
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
    use std::sync::Mutex;

    static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    fn record(url: &str) -> Result<(), String> {
        OPENED.lock().unwrap().push(url.to_string());
        Ok(())
    }
    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }
    fn screen(v: &mut TodoView, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
            .collect()
    }

    /// Item 1 doing (session + task), 2 open, 3 done.
    fn seeded() -> (tempfile::TempDir, TodoView) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("todo.json");
        todo::update(&p, |t| {
            let a = t.add(
                "Permission error",
                Some("https://app.example/tasks/7"),
                Some(("sess-a", "work-2")),
            );
            let _ = a;
            t.add("Look at Tini's PR", None, None);
            let c = t.add("Fix login", None, Some(("sess-c", "api-fix")));
            t.done(c)
        })
        .unwrap();
        let mut v = TodoView::at(p);
        v.opener = record;
        v.on_show();
        (d, v)
    }

    #[test]
    fn the_list_shows_doing_then_open_then_done() {
        let (_d, mut v) = seeded();
        let s = screen(&mut v, 100, 10);
        let (a, b, c) = (
            s.find("● Permission error").unwrap(),
            s.find("○ Look at Tini's PR").unwrap(),
            s.find("✓ Fix login").unwrap(),
        );
        assert!(a < b && b < c, "{s}");
        assert!(s.contains("⧉ work-2") && s.contains("↗ task"), "{s}");
    }

    #[test]
    fn keys_add_toggle_delete_open_and_start() {
        let (d, mut v) = seeded();
        let p = d.path().join("todo.json");
        // Enter on an item with a task opens it in the browser.
        assert_eq!(v.key(k(KeyCode::Enter)), Outcome::None);
        assert!(
            OPENED
                .lock()
                .unwrap()
                .contains(&"https://app.example/tasks/7".to_string())
        );
        // `g` goes to its session.
        assert_eq!(
            v.key(k(KeyCode::Char('g'))),
            Outcome::Session("sess-a".into())
        );
        // `c` on the open item starts a session that claims it.
        v.key(k(KeyCode::Char('j')));
        match v.key(k(KeyCode::Char('c'))) {
            Outcome::Spawn(prompt) => assert!(prompt.contains("bearclaw todo claim 2"), "{prompt}"),
            other => panic!("{other:?}"),
        }
        // `x` toggles done.
        v.key(k(KeyCode::Char('x')));
        assert_eq!(todo::load(&p).unwrap().get(2).unwrap().status, Status::Done);
        // `n` adds one of yours.
        v.key(k(KeyCode::Char('n')));
        assert!(v.typing());
        for c in "Write docs".chars() {
            v.key(k(KeyCode::Char(c)));
        }
        v.key(k(KeyCode::Enter));
        let t = todo::load(&p).unwrap();
        let added = t.items.iter().find(|i| i.text == "Write docs").unwrap();
        assert_eq!(added.status, Status::Open);
        // `d` deletes after a `y`.
        let before = t.items.len();
        v.key(k(KeyCode::Char('d')));
        v.key(k(KeyCode::Char('y')));
        assert_eq!(todo::load(&p).unwrap().items.len(), before - 1);
    }
}
