//! The Tasks tab: your open Productive tasks as a kanban, like the PRs tab — a lane per status
//! in the board's order (or per project, `g`), a card per task.

use std::collections::HashMap;
use std::time::Duration;

use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use super::loader::Loader;
use super::pager::wrap;
use super::{Outcome, View, common_key};
use crate::cli::render::{ago, fit};
use crate::core::productive::{self, Grouping, Task};
use crate::core::tools::open_url;

const TTL: Duration = Duration::from_secs(120);
/// The narrowest a lane gets before lanes page instead.
const LANE_MIN: u16 = 26;
/// Rows one card takes: border, two title lines, `#N` and project, due date and age, border.
const CARD_H: u16 = 6;

pub struct TasksView {
    pub(crate) load: Loader<Vec<Task>>,
    grouping: Grouping,
    /// The selected lane (index into [`TasksView::lanes`]).
    lane: usize,
    /// The selected card per lane, by the lane's name — survives refreshes and lane hops.
    card: HashMap<String, usize>,
    /// The leftmost lane on screen when they don't all fit.
    first_lane: usize,
    /// Where each card was last drawn, with its index into the tasks — for clicks.
    hits: Vec<(Rect, usize)>,
    /// How a task's link opens — swapped out in tests, which must not launch a browser.
    pub(crate) opener: fn(&str) -> Result<(), String>,
}

impl Default for TasksView {
    fn default() -> Self {
        Self::new()
    }
}

/// `02.01.` and how it reads against today (the web app's rule): overdue red, due within
/// three days yellow.
fn due_cell(due: Option<&str>, today: NaiveDate) -> (String, Color) {
    let Some(d) = due.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) else {
        return (String::new(), Color::DarkGray);
    };
    let color = match (d - today).num_days() {
        ..0 => Color::Red,
        0..=3 => Color::Yellow,
        _ => Color::Gray,
    };
    (d.format("%d.%m.").to_string(), color)
}

impl TasksView {
    pub fn new() -> Self {
        TasksView {
            load: Loader::new(),
            grouping: Grouping::Status,
            lane: 0,
            card: HashMap::new(),
            first_lane: 0,
            hits: Vec::new(),
            opener: open_url,
        }
    }

    fn fetch(&mut self) {
        self.load
            .start(|| productive::Settings::load().and_then(|s| productive::my_open_tasks(&s)));
    }

    fn tasks(&self) -> &[Task] {
        self.load.data.as_deref().unwrap_or(&[])
    }

    /// The lanes: a name and the tasks in it, in the board's order (or by project).
    fn lanes(&self) -> Vec<(String, Vec<usize>)> {
        productive::group(self.tasks(), self.grouping)
            .into_iter()
            .map(|(name, idx)| (if name.is_empty() { "—".into() } else { name }, idx))
            .collect()
    }

    pub fn selected_task(&self) -> Option<&Task> {
        let lanes = self.lanes();
        let (name, idx) = lanes.get(self.lane.min(lanes.len().saturating_sub(1)))?;
        let at = self
            .card
            .get(name)
            .copied()
            .unwrap_or(0)
            .min(idx.len().checked_sub(1)?);
        Some(&self.tasks()[idx[at]])
    }

    /// Select the card of `tasks()[i]`.
    fn select(&mut self, i: usize) {
        for (n, (name, idx)) in self.lanes().into_iter().enumerate() {
            if let Some(at) = idx.iter().position(|&x| x == i) {
                self.lane = n;
                self.card.insert(name, at);
                return;
            }
        }
    }

    fn step_card(&mut self, down: bool) {
        let lanes = self.lanes();
        let Some((name, idx)) = lanes.get(self.lane) else {
            return;
        };
        let at = self.card.entry(name.clone()).or_insert(0);
        *at = if down {
            (*at + 1).min(idx.len().saturating_sub(1))
        } else {
            at.saturating_sub(1)
        };
    }

    /// The task in Productive, in the browser.
    fn open(&self) -> Outcome {
        match self.selected_task().map(|t| (self.opener)(&t.url)) {
            Some(Err(e)) => Outcome::Log(e),
            _ => Outcome::None,
        }
    }

    /// A card: the title, `#N` with what the lane doesn't say (the project by status, the status
    /// by project), the due date and how long ago it changed. The border is the due date's colour.
    fn card(
        t: &Task,
        selected: bool,
        width: u16,
        today: NaiveDate,
        grouping: Grouping,
    ) -> (Vec<Line<'static>>, Style) {
        let w = width.saturating_sub(2) as usize;
        let mut title = wrap(&t.title, w);
        if title.len() > 2 {
            title[1] = fit(&format!("{} …", title[1]), w);
            title.truncate(2);
        }
        let bold = if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        let mut lines: Vec<Line> = title
            .into_iter()
            .map(|l| Line::styled(l, Style::default().add_modifier(bold)))
            .collect();
        while lines.len() < 2 {
            lines.push(Line::default());
        }
        let (other, other_color) = match grouping {
            Grouping::Status => (&t.project, Color::Gray),
            Grouping::Project => (&t.status, Color::Magenta),
        };
        let num = format!("#{} ", t.number);
        lines.push(Line::from(vec![
            Span::styled(num.clone(), Style::default().fg(Color::Cyan)),
            Span::styled(
                fit(other, w.saturating_sub(num.chars().count())),
                Style::default().fg(other_color),
            ),
        ]));
        let (due, due_color) = due_cell(t.due_date.as_deref(), today);
        let due = if due.is_empty() {
            "no due date".to_string()
        } else {
            format!("due {due}")
        };
        let updated = chrono::DateTime::parse_from_rfc3339(&t.updated_at)
            .map(|d| format!("  {} ago", ago(Some(d.timestamp_millis()))))
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(due, Style::default().fg(due_color)),
            Span::styled(updated, Style::default().fg(Color::DarkGray)),
        ]));
        let border = Style::default().fg(match due_color {
            Color::Red | Color::Yellow => due_color,
            _ => Color::DarkGray,
        });
        let border = if selected {
            border.add_modifier(Modifier::BOLD)
        } else {
            border
        };
        (lines, border)
    }

    fn draw_lane(
        &self,
        f: &mut Frame,
        area: Rect,
        (name, idx): (&str, &[usize]),
        selected_lane: bool,
        (more_left, more_right): (bool, bool),
    ) -> Vec<(Rect, usize)> {
        let left = if more_left { "‹ " } else { "" };
        let right = if more_right { " ›" } else { "" };
        let border = if selected_lane {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .title(fit(
                &format!("{left} {name} ({}) {right}", idx.len()),
                area.width.saturating_sub(2) as usize,
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let mut hits = Vec::new();
        if inner.height == 0 || inner.width == 0 || idx.is_empty() {
            return hits;
        }
        let at = self.card.get(name).copied().unwrap_or(0).min(idx.len() - 1);
        // Leave a row for "+N more ↓" when the cards don't all fit.
        let fits = (inner.height / CARD_H).max(1) as usize;
        let shown = if idx.len() > fits {
            ((inner.height.saturating_sub(1)) / CARD_H).max(1) as usize
        } else {
            fits
        };
        let offset = (at + 1).saturating_sub(shown);
        let today = chrono::Local::now().date_naive();
        let mut y = inner.y;
        for (n, &i) in idx.iter().enumerate().skip(offset).take(shown) {
            let h = CARD_H.min(inner.y + inner.height - y);
            if h < 3 {
                break;
            }
            let selected = selected_lane && n == at;
            let (lines, style) = Self::card(
                &self.tasks()[i],
                selected,
                inner.width,
                today,
                self.grouping,
            );
            let card = Block::default()
                .borders(Borders::ALL)
                .border_type(if selected {
                    BorderType::Thick
                } else {
                    BorderType::Rounded
                })
                .border_style(style);
            let rect = Rect::new(inner.x, y, inner.width, h);
            f.render_widget(Paragraph::new(lines).block(card), rect);
            hits.push((rect, i));
            y += h;
        }
        let below = idx.len().saturating_sub(offset + shown);
        if below > 0 && y < inner.y + inner.height {
            f.render_widget(
                Paragraph::new(Line::styled(
                    format!("+{below} more ↓"),
                    Style::default().fg(Color::DarkGray),
                )),
                Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
            );
        }
        hits
    }
}

impl View for TasksView {
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
            self.lane = self.lane.min(self.lanes().len().saturating_sub(1));
        }
    }

    fn click(&mut self, col: u16, row: u16) -> Outcome {
        let hit =
            self.hits.iter().copied().find(|(r, _)| {
                col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
            });
        if let Some((_, i)) = hit {
            self.select(i);
        }
        Outcome::None
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
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
                " ←/→ lane · ↑/↓ card · ⏎ open in Productive · g group by status/project · r refresh · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
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
        let lanes = self.lanes();
        if lanes.is_empty() {
            f.render_widget(Paragraph::new(" no open tasks"), inner);
            return;
        }
        let visible = ((inner.width / LANE_MIN).max(1) as usize).min(lanes.len());
        self.lane = self.lane.min(lanes.len() - 1);
        if self.lane < self.first_lane {
            self.first_lane = self.lane;
        }
        if self.lane >= self.first_lane + visible {
            self.first_lane = self.lane + 1 - visible;
        }
        self.first_lane = self.first_lane.min(lanes.len() - visible);
        let lane_w = inner.width / visible as u16;
        let mut hits = Vec::new();
        for (n, (name, idx)) in lanes.iter().enumerate().skip(self.first_lane).take(visible) {
            let slot = (n - self.first_lane) as u16;
            let x = inner.x + slot * lane_w;
            // The last lane takes the rounding remainder.
            let w = if slot as usize == visible - 1 {
                inner.width - slot * lane_w
            } else {
                lane_w
            };
            let edges = (
                slot == 0 && self.first_lane > 0,
                slot as usize == visible - 1 && self.first_lane + visible < lanes.len(),
            );
            hits.extend(self.draw_lane(
                f,
                Rect::new(x, inner.y, w, inner.height),
                (name, idx),
                n == self.lane,
                edges,
            ));
        }
        self.hits = hits;
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if let Some(o) = common_key(k) {
            return o;
        }
        match k.code {
            KeyCode::Right | KeyCode::Char('l') => {
                let n = self.lanes().len();
                self.lane = (self.lane + 1).min(n.saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Char('h') => self.lane = self.lane.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.step_card(true),
            KeyCode::Up | KeyCode::Char('k') => self.step_card(false),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('g') => {
                self.grouping = match self.grouping {
                    Grouping::Status => Grouping::Project,
                    Grouping::Project => Grouping::Status,
                };
                self.lane = 0;
                self.first_lane = 0;
                self.card.clear();
            }
            KeyCode::Enter | KeyCode::Char('o') => return self.open(),
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

    fn draw(v: &mut TasksView, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        t.backend().buffer().clone()
    }

    fn screen(v: &mut TasksView, w: u16, h: u16) -> String {
        let buf = draw(v, w, h);
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn key(v: &mut TasksView, c: KeyCode) -> Outcome {
        v.key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    fn task(n: &str, status: &str, rank: (u8, i64), project: &str) -> Task {
        Task {
            id: n.into(),
            number: n.into(),
            title: format!("task {n}"),
            status: status.into(),
            status_rank: rank,
            project: project.into(),
            url: format!("https://app.example/tasks/{n}"),
            ..Default::default()
        }
    }

    fn number(v: &TasksView) -> Option<String> {
        v.selected_task().map(|t| t.number.clone())
    }

    #[test]
    fn tasks_tab_shows_errors_inline() {
        let mut v = TasksView::new();
        v.load.error = Some("set productive.organizationId: bearclaw config set".into());
        assert!(screen(&mut v, 80, 10).contains("set productive.organizationId"));
        assert_eq!(key(&mut v, KeyCode::Char('j')), Outcome::None);
        assert_eq!(key(&mut v, KeyCode::Enter), Outcome::None);
    }

    #[test]
    fn a_lane_per_status_in_board_order_and_g_makes_them_projects() {
        let mut v = TasksView::new();
        v.load.data = Some(vec![
            task("1", "Working", (2, 4), "Web"),
            task("2", "Triage", (1, 1), "Api"),
            task("3", "Working", (2, 4), "Api"),
        ]);
        let s = screen(&mut v, 100, 20);
        let lanes = s.lines().find(|l| l.contains("Triage (1)")).unwrap_or("");
        assert!(
            lanes.contains("Working (2)")
                && lanes.find("Triage (1)").unwrap() < lanes.find("Working (2)").unwrap(),
            "{s}"
        );
        assert!(
            s.contains("#2") && s.contains("task 2") && s.contains("Api"),
            "{s}"
        );
        assert_eq!(number(&v).as_deref(), Some("2"));
        key(&mut v, KeyCode::Right);
        assert_eq!(number(&v).as_deref(), Some("1"));
        key(&mut v, KeyCode::Down);
        assert_eq!(number(&v).as_deref(), Some("3"));
        key(&mut v, KeyCode::Down);
        assert_eq!(number(&v).as_deref(), Some("3"));
        key(&mut v, KeyCode::Left);
        assert_eq!(number(&v).as_deref(), Some("2"));
        // Back on the lane, its card is where it was left.
        key(&mut v, KeyCode::Char('l'));
        assert_eq!(number(&v).as_deref(), Some("3"));

        key(&mut v, KeyCode::Char('g'));
        let p = screen(&mut v, 100, 20);
        assert!(p.contains("Api (2)") && p.contains("Web (1)"), "{p}");
        // A card names what its lane doesn't: the status, by project.
        assert!(p.contains("Triage"), "{p}");
    }

    #[test]
    fn narrow_screens_page_the_lanes() {
        let mut v = TasksView::new();
        v.load.data = Some(vec![
            task("1", "Triage", (1, 1), "Web"),
            task("2", "Working", (2, 1), "Web"),
            task("3", "Review", (2, 2), "Web"),
        ]);
        let s = screen(&mut v, 60, 14);
        assert!(
            s.contains("Triage (1)") && s.contains("›") && !s.contains("Review (1)"),
            "{s}"
        );
        key(&mut v, KeyCode::Right);
        key(&mut v, KeyCode::Right);
        let s = screen(&mut v, 60, 14);
        assert!(
            s.contains("Review (1)") && s.contains("‹") && !s.contains("Triage (1)"),
            "{s}"
        );
    }

    #[test]
    fn a_card_is_red_when_overdue_and_yellow_when_due_within_three_days() {
        let today = chrono::Local::now().date_naive();
        let day = |d: i64| {
            Some(
                (today + chrono::Duration::days(d))
                    .format("%Y-%m-%d")
                    .to_string(),
            )
        };
        let mut v = TasksView::new();
        v.load.data = Some(vec![
            Task {
                due_date: day(-1),
                ..task("1", "Triage", (1, 1), "Web")
            },
            Task {
                due_date: day(2),
                ..task("2", "Working", (2, 1), "Web")
            },
            Task {
                due_date: day(9),
                ..task("3", "Review", (2, 2), "Web")
            },
        ]);
        let buf = draw(&mut v, 120, 14);
        // Row 2 is where the cards' top edges are, one per lane.
        let corners: Vec<Color> = (0..120)
            .filter(|&x| ["╭", "┏"].contains(&buf[(x, 2)].symbol()))
            .map(|x| buf[(x, 2)].fg)
            .collect();
        assert_eq!(corners, [Color::Red, Color::Yellow, Color::DarkGray]);
    }

    static OPENED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn record(url: &str) -> Result<(), String> {
        OPENED.lock().unwrap().push(url.to_string());
        Ok(())
    }

    #[test]
    fn enter_opens_the_task_in_the_browser() {
        let mut v = TasksView::new();
        v.opener = record;
        v.load.data = Some(vec![Task {
            id: "7".into(),
            number: "12".into(),
            title: "Fix login".into(),
            project: "Website".into(),
            status: "In progress".into(),
            due_date: Some("2000-01-02".into()),
            url: "https://app.example/tasks/7".into(),
            ..Default::default()
        }]);
        let board = screen(&mut v, 100, 10);
        assert!(
            board.contains("#12") && board.contains("Fix login") && board.contains("Website"),
            "{board}"
        );
        assert!(board.contains("02.01."), "{board}");
        assert_eq!(key(&mut v, KeyCode::Enter), Outcome::None);
        assert!(
            OPENED
                .lock()
                .unwrap()
                .contains(&"https://app.example/tasks/7".to_string())
        );
        // No summary page: the board stays, and Esc leaves the tab.
        assert!(screen(&mut v, 100, 10).contains("#12"));
        assert_eq!(key(&mut v, KeyCode::Esc), Outcome::Home);
    }

    #[test]
    fn a_click_selects_the_card_under_it() {
        let mut v = TasksView::new();
        v.load.data = Some(vec![
            task("1", "Triage", (1, 1), "Web"),
            task("2", "Working", (2, 1), "Web"),
            task("3", "Working", (2, 1), "Api"),
        ]);
        let s = screen(&mut v, 100, 20);
        let (row, line) = s
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains("task 3"))
            .unwrap();
        let col = line[..line.find("task 3").unwrap()].chars().count() as u16;
        assert_eq!(v.click(col, row as u16), Outcome::None);
        assert_eq!(number(&v).as_deref(), Some("3"));
    }
}
