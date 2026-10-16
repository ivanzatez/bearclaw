//! The PRs tab: `bearclaw prs` as a kanban — one lane per column, one card per PR.

use std::collections::HashMap;
use std::time::Duration;

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
use crate::core::prs::{self, Column, Mode, Pr, View as PrView};
use crate::core::tools::open_url;

/// `prs.refresh` decides when `Mode::Auto` really fetches; this only bounds how often we ask.
const TTL: Duration = Duration::from_secs(60);
/// The narrowest a lane gets before lanes page instead.
const LANE_MIN: u16 = 26;
/// Rows one card takes: border, two title lines, `repo#N`, meta, border.
const CARD_H: u16 = 6;

/// The lanes, in tb-pr's order and wording. "Waiting on author" has no lane.
const LANES: [(&str, Column); 5] = [
    ("Draft", Column::DraftMine),
    ("In review", Column::ReviewMine),
    ("Ready to merge", Column::ReadyToMergeMine),
    ("Waiting on me", Column::WaitingOnMe),
    ("Mentions", Column::UnseenMentions),
];

/// Every lane, empty ones included, each with its PRs (indices into `prs`).
pub fn lanes(prs: &[Pr]) -> Vec<(&'static str, Column, Vec<usize>)> {
    LANES
        .into_iter()
        .map(|(title, col)| {
            let idx: Vec<usize> = (0..prs.len()).filter(|&i| prs[i].column == col).collect();
            (title, col, idx)
        })
        .collect()
}

pub struct PrsView {
    pub(crate) load: Loader<PrView>,
    /// The selected lane (index into [`lanes`]).
    lane: usize,
    /// The selected card per column — survives refreshes and lane hops.
    card: HashMap<Column, usize>,
    /// The leftmost lane on screen when they don't all fit.
    first_lane: usize,
}

impl Default for PrsView {
    fn default() -> Self {
        Self::new()
    }
}

fn checks_mark(p: &Pr) -> (&'static str, Color) {
    match p.checks.as_deref() {
        Some("success") => ("✓ ", Color::Green),
        Some("failure" | "error") => ("✗ ", Color::Red),
        Some(_) => ("● ", Color::Yellow),
        None => ("", Color::Gray),
    }
}

impl PrsView {
    pub fn new() -> Self {
        PrsView {
            load: Loader::new(),
            lane: 0,
            card: HashMap::new(),
            first_lane: 0,
        }
    }

    fn fetch(&mut self, mode: Mode) {
        self.load.start(move || {
            let now = chrono::Utc::now().timestamp_millis();
            prs::get(&prs::Settings::load(), mode, now).map_err(|e| e.to_string())
        });
    }

    fn prs(&self) -> &[Pr] {
        self.load.data.as_ref().map_or(&[], |v| &v.snap.prs)
    }

    pub fn selected_pr(&self) -> Option<&Pr> {
        let lanes = lanes(self.prs());
        let (_, col, idx) = lanes.get(self.lane.min(lanes.len().saturating_sub(1)))?;
        let at = self
            .card
            .get(col)
            .copied()
            .unwrap_or(0)
            .min(idx.len().checked_sub(1)?);
        Some(&self.prs()[idx[at]])
    }

    fn step_card(&mut self, down: bool) {
        let lanes = lanes(self.prs());
        let Some((_, col, idx)) = lanes.get(self.lane) else {
            return;
        };
        let at = self.card.entry(*col).or_insert(0);
        *at = if down {
            (*at + 1).min(idx.len().saturating_sub(1))
        } else {
            at.saturating_sub(1)
        };
    }

    fn card(p: &Pr, selected: bool, width: u16) -> (Vec<Line<'static>>, Style) {
        let (mark, color) = checks_mark(p);
        let w = width.saturating_sub(2) as usize;
        let mut title = wrap(&format!("{mark}{}", p.title), w);
        if title.len() > 2 {
            title[1] = fit(&format!("{}{}", title[1], " …"), w);
            title.truncate(2);
        }
        let bold = if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        let mut lines: Vec<Line> = title
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                // The check mark keeps its colour on the first line.
                if i == 0 && !mark.is_empty() && t.starts_with(mark.trim_end()) {
                    let rest = t[mark.len().min(t.len())..].to_string();
                    Line::from(vec![
                        Span::styled(mark, Style::default().fg(color)),
                        Span::styled(rest, Style::default().add_modifier(bold)),
                    ])
                } else {
                    Line::styled(t, Style::default().add_modifier(bold))
                }
            })
            .collect();
        while lines.len() < 2 {
            lines.push(Line::default());
        }
        let draft = if p.draft { " draft" } else { "" };
        lines.push(Line::styled(
            fit(&format!("{}#{}{draft}", p.repo, p.number), w),
            Style::default().fg(Color::Cyan),
        ));
        let comments = if p.comments > 0 {
            format!("  💬 {}", p.comments)
        } else {
            String::new()
        };
        lines.push(Line::styled(
            fit(
                &format!("{}  {}{comments}", p.size, ago(Some(p.updated_at))),
                w,
            ),
            Style::default().fg(Color::Gray),
        ));
        let border = if selected {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        (lines, Style::default().fg(border))
    }

    fn draw_lane(
        &self,
        f: &mut Frame,
        area: Rect,
        (title, col, idx): &(&str, Column, Vec<usize>),
        selected_lane: bool,
        (more_left, more_right): (bool, bool),
    ) {
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
                &format!("{left} {title} ({}) {right}", idx.len()),
                area.width.saturating_sub(2) as usize,
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        if idx.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled(
                    "(inbox zero)",
                    Style::default().fg(Color::DarkGray),
                )),
                inner,
            );
            return;
        }
        let at = self.card.get(col).copied().unwrap_or(0).min(idx.len() - 1);
        // Leave a row for "+N more ↓" when the cards don't all fit.
        let fits = (inner.height / CARD_H).max(1) as usize;
        let shown = if idx.len() > fits {
            ((inner.height.saturating_sub(1)) / CARD_H).max(1) as usize
        } else {
            fits
        };
        let offset = (at + 1).saturating_sub(shown);
        let mut y = inner.y;
        for (n, &i) in idx.iter().enumerate().skip(offset).take(shown) {
            let h = CARD_H.min(inner.y + inner.height - y);
            if h < 3 {
                break;
            }
            let selected = selected_lane && n == at;
            let (lines, style) = Self::card(&self.prs()[i], selected, inner.width);
            let card = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(style);
            f.render_widget(
                Paragraph::new(lines).block(card),
                Rect::new(inner.x, y, inner.width, h),
            );
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
    }
}

impl View for PrsView {
    fn on_show(&mut self) {
        if self.load.stale(TTL) {
            self.fetch(Mode::Auto);
        }
    }

    fn tick(&mut self) {
        if self.load.poll() {
            self.lane = self.lane.min(lanes(self.prs()).len().saturating_sub(1));
        }
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        let mut title = String::from(" PRs ");
        if let Some(v) = &self.load.data {
            title = format!(" PRs · @{} · {} ", v.snap.user, v.snap.prs.len());
            if v.stale {
                title.push_str(&format!(
                    "· cached{} ",
                    v.error
                        .as_deref()
                        .map(|e| format!(" — {e}"))
                        .unwrap_or_default()
                ));
            }
        }
        if self.load.loading {
            title.push_str("· loading… ");
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(fit(&title, area.width.saturating_sub(2) as usize))
            .title_bottom(Line::styled(
                " ←/→ lane · ↑/↓ card · ⏎/o open · r refresh · esc back ",
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
                (None, true) => "asking GitHub…".into(),
                (None, false) => "no pull requests yet — r to fetch".into(),
            };
            f.render_widget(
                Paragraph::new(format!(" {msg}")).wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        let lanes = lanes(self.prs());
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
        for (n, lane) in lanes.iter().enumerate().skip(self.first_lane).take(visible) {
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
            self.draw_lane(
                f,
                Rect::new(x, inner.y, w, inner.height),
                lane,
                n == self.lane,
                edges,
            );
        }
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if let Some(o) = common_key(k) {
            return o;
        }
        match k.code {
            KeyCode::Right | KeyCode::Char('l') => {
                let n = lanes(self.prs()).len();
                self.lane = (self.lane + 1).min(n.saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Char('h') => self.lane = self.lane.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.step_card(true),
            KeyCode::Up | KeyCode::Char('k') => self.step_card(false),
            KeyCode::Char('r') => self.fetch(Mode::Refresh),
            KeyCode::Enter | KeyCode::Char('o') => {
                if let Some(p) = self.selected_pr()
                    && let Err(e) = open_url(&p.url)
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
    use crate::core::prs::{Column, Pr, Rotting, Snapshot, View as PrView};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn pr(column: Column, number: u64, title: &str) -> Pr {
        Pr {
            column,
            owner: "acme".into(),
            repo: "api".into(),
            number,
            title: title.into(),
            url: format!("https://github.com/acme/api/pull/{number}"),
            author: "dev".into(),
            draft: false,
            created_at: 0,
            updated_at: 0,
            additions: 1,
            deletions: 1,
            changed_files: 1,
            size: "XS".into(),
            base_branch: "main".into(),
            head_branch: "x".into(),
            cross_repository: false,
            head_oid: None,
            review_decision: None,
            checks: Some("success".into()),
            comments: 0,
            requested_reviewers: vec![],
            my_review: None,
            mention: None,
            rotting: Rotting::default(),
            local: None,
        }
    }

    fn view_with(prs: Vec<Pr>) -> PrsView {
        let mut v = PrsView::new();
        v.load.data = Some(PrView {
            snap: Snapshot {
                user: "me".into(),
                prs,
                ..Default::default()
            },
            stale: false,
            error: None,
        });
        v
    }

    fn screen(v: &mut PrsView, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn key(v: &mut PrsView, c: KeyCode) {
        v.key(KeyEvent::new(c, KeyModifiers::NONE));
    }

    #[test]
    fn lanes_follow_the_tb_pr_order_empty_ones_included() {
        let prs = vec![
            pr(Column::WaitingOnMe, 1, "review me"),
            pr(Column::DraftMine, 2, "draft thing"),
            pr(Column::WaitingOnAuthor, 3, "their turn"),
        ];
        let got: Vec<(&str, Vec<usize>)> = lanes(&prs)
            .into_iter()
            .map(|(title, _, idx)| (title, idx))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Draft", vec![1]),
                ("In review", vec![]),
                ("Ready to merge", vec![]),
                ("Waiting on me", vec![0]),
                ("Mentions", vec![]),
            ]
        );
    }

    #[test]
    fn kanban_moves_between_lanes_and_cards() {
        let mut v = view_with(vec![
            pr(Column::WaitingOnMe, 1, "review me"),
            pr(Column::DraftMine, 2, "draft a"),
            pr(Column::DraftMine, 3, "draft b"),
        ]);
        let n = |v: &PrsView| v.selected_pr().map(|p| p.number);
        assert_eq!(n(&v), Some(2));
        key(&mut v, KeyCode::Down);
        assert_eq!(n(&v), Some(3));
        key(&mut v, KeyCode::Down);
        assert_eq!(n(&v), Some(3));
        // An empty lane selects nothing and survives every key.
        key(&mut v, KeyCode::Right);
        assert_eq!(n(&v), None);
        key(&mut v, KeyCode::Down);
        key(&mut v, KeyCode::Enter);
        for _ in 0..2 {
            key(&mut v, KeyCode::Right);
        }
        assert_eq!(n(&v), Some(1));
        for _ in 0..5 {
            key(&mut v, KeyCode::Right);
        }
        assert_eq!(n(&v), None);
        for _ in 0..4 {
            key(&mut v, KeyCode::Left);
        }
        // Each lane remembers its card.
        assert_eq!(n(&v), Some(3));
    }

    #[test]
    fn kanban_draws_cards_and_pages_on_narrow_screens() {
        let mut v = view_with(vec![
            pr(Column::WaitingOnMe, 1, "review me"),
            pr(Column::DraftMine, 2, "draft a"),
            pr(Column::DraftMine, 3, "draft b"),
        ]);
        let wide = screen(&mut v, 160, 30);
        for lane in [
            "Draft (2)",
            "In review (0)",
            "Ready to merge (0)",
            "Waiting on me (1)",
            "Mentions (0)",
        ] {
            assert!(wide.contains(lane), "{lane}: {wide}");
        }
        assert!(
            wide.contains("api#1") && wide.contains("review me") && wide.contains("✓"),
            "{wide}"
        );
        assert!(wide.contains("inbox zero"), "{wide}");
        let narrow = screen(&mut v, 40, 30);
        assert!(
            narrow.contains("Draft (2)") && !narrow.contains("In review"),
            "{narrow}"
        );
        assert!(narrow.contains("›"), "{narrow}");
        key(&mut v, KeyCode::Right);
        let paged = screen(&mut v, 40, 30);
        assert!(
            paged.contains("In review (0)") && paged.contains("‹"),
            "{paged}"
        );
        // Tiny areas draw without panicking.
        for (w, h) in [(0, 0), (1, 1), (10, 3), (26, 5)] {
            screen(&mut v, w, h);
        }
    }

    #[test]
    fn an_error_with_nothing_cached_is_shown() {
        let mut v = PrsView::new();
        v.load.error = Some("gh: not logged in".into());
        assert!(screen(&mut v, 80, 10).contains("gh: not logged in"));
        assert_eq!(
            v.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            Outcome::None
        );
        assert_eq!(
            v.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Outcome::Home
        );
    }
}
