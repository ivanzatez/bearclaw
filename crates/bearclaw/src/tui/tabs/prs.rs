//! The PRs tab: `bearclaw prs` as a kanban — one lane per column, one card per PR.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use super::loader::Loader;
use super::pager::wrap;
use super::{CardStat, Outcome, View, common_key};
use crate::cli::render::{ago, fit};
use crate::core::prs::{self, Column, Mode, Pr, Rotting, View as PrView};
use crate::core::tbpr;
use crate::core::tools::open_url;

/// `prs.refresh` decides when `Mode::Auto` really fetches; this only bounds how often we ask —
/// both on `on_show` and the periodic pull while the tab is on screen.
const TTL: Duration = Duration::from_secs(60);
/// After `Enter` opens a PR, keep doing real `tb-pr refresh`es for this long, so a PR merged or
/// closed in the browser drops off the board within a minute of coming back.
const WATCH: Duration = Duration::from_secs(90);
/// How often those post-open refreshes fire — tight enough to clear a merge "within a minute".
const WATCH_PULL: Duration = Duration::from_secs(20);
/// Drawn more recently than this ⇒ the tab is on screen (only the active tab draws, and the
/// loop redraws several times a second).
const ON_SCREEN: Duration = Duration::from_millis(750);
/// The narrowest a lane gets before lanes page instead.
const LANE_MIN: u16 = 26;
/// Rows one card takes: border, two title lines, `repo#N`, meta, border.
const CARD_H: u16 = 6;

/// One kanban lane: its title, column, the PRs it shows (indices into `prs`) and how many
/// drafts the draft filter hides from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub title: String,
    pub column: Column,
    pub idx: Vec<usize>,
    pub hidden_drafts: usize,
}

/// The lanes in tb-pr's order and wording, empty ones included. Drafts asking for your review
/// (or waiting on their author) stay hidden while `hide_drafts`; "Waiting on author" has a lane
/// only with `show_author` — both tb-pr's defaults and its `D` / `A` keys.
pub fn lanes(prs: &[Pr], hide_drafts: bool, show_author: bool) -> Vec<Lane> {
    let order = [
        ("Draft", Column::DraftMine, true),
        ("In review", Column::ReviewMine, true),
        ("Ready to merge", Column::ReadyToMergeMine, true),
        ("Waiting on me", Column::WaitingOnMe, true),
        ("Waiting on author", Column::WaitingOnAuthor, show_author),
        ("Mentions", Column::UnseenMentions, true),
    ];
    order
        .into_iter()
        .filter(|(_, _, shown)| *shown)
        .map(|(title, column, _)| {
            let filtered =
                hide_drafts && matches!(column, Column::WaitingOnMe | Column::WaitingOnAuthor);
            let all: Vec<usize> = (0..prs.len())
                .filter(|&i| prs[i].column == column)
                .collect();
            let idx: Vec<usize> = all
                .iter()
                .copied()
                .filter(|&i| !(filtered && prs[i].draft))
                .collect();
            Lane {
                title: title.to_string(),
                column,
                hidden_drafts: all.len() - idx.len(),
                idx,
            }
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
    /// tb-pr's `D`: drafts asking for your review stay out of sight (on by default).
    hide_drafts: bool,
    /// tb-pr's `A`: the "Waiting on author" lane (off by default).
    show_author: bool,
    /// `<app>/<org slug>` for task links; read from the config when first needed.
    pub(crate) task_base: Option<String>,
    /// How links open — swapped out in tests, which must not launch a browser.
    pub(crate) opener: fn(&str) -> Result<(), String>,
    /// When `draw` last ran. Only the active tab draws, so a recent stamp means we are on
    /// screen and may pull on our own.
    shown_at: Option<Instant>,
    /// Until when the periodic pull should be a real `tb-pr refresh` — set when a PR is opened.
    watch_until: Option<Instant>,
}

impl Default for PrsView {
    fn default() -> Self {
        Self::new()
    }
}

/// Put `text` on the clipboard (`pbcopy`, else `wl-copy` / `xclip`).
fn copy(text: &str) -> Result<(), String> {
    crate::tui::term::clip::local(text)
}

/// How long a PR has waited for its column, as a traffic light: fresh green, warming or stale
/// yellow, rotting or worse red (the thresholds are per column, `prs::rotting`).
pub fn rot_color(r: Rotting) -> Color {
    match r {
        Rotting::Fresh => Color::Green,
        Rotting::Warming | Rotting::Stale => Color::Yellow,
        Rotting::Rotting | Rotting::Critical => Color::Red,
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
            hide_drafts: true,
            show_author: false,
            task_base: None,
            opener: open_url,
            shown_at: None,
            watch_until: None,
        }
    }

    /// The tab is on screen right now (drawn within the last [`ON_SCREEN`]).
    fn on_screen(&self) -> bool {
        self.shown_at.is_some_and(|t| t.elapsed() < ON_SCREEN)
    }

    /// We are inside the post-open window where pulls should force a real refresh.
    fn watching(&self) -> bool {
        self.watch_until.is_some_and(|t| Instant::now() < t)
    }

    /// Whether `tick` should pull now, and in which mode — `None` to stay put. While the tab is
    /// on screen we keep the board fresh on our own: a cheap cached pull normally, a real
    /// `tb-pr refresh` during the window after a PR was opened.
    fn pull_mode(&self) -> Option<Mode> {
        if !self.on_screen() {
            return None;
        }
        let (mode, every) = if self.watching() {
            (Mode::Refresh, WATCH_PULL)
        } else {
            (Mode::Auto, TTL)
        };
        self.load.stale(every).then_some(mode)
    }

    fn fetch(&mut self, mode: Mode) {
        self.load.start(move || {
            // tb-pr's board when it is installed (its rules, exactly); bearclaw's own otherwise.
            if let Some(bin) = tbpr::binary() {
                return tbpr::fetch(&bin, mode == Mode::Refresh);
            }
            let now = chrono::Utc::now().timestamp_millis();
            prs::get(&prs::Settings::load(), mode, now).map_err(|e| e.to_string())
        });
    }

    fn prs(&self) -> &[Pr] {
        self.load.data.as_ref().map_or(&[], |v| &v.snap.prs)
    }

    fn lanes(&self) -> Vec<Lane> {
        lanes(self.prs(), self.hide_drafts, self.show_author)
    }

    pub fn selected_pr(&self) -> Option<&Pr> {
        let lanes = self.lanes();
        let lane = lanes.get(self.lane.min(lanes.len().saturating_sub(1)))?;
        let at = self
            .card
            .get(&lane.column)
            .copied()
            .unwrap_or(0)
            .min(lane.idx.len().checked_sub(1)?);
        Some(&self.prs()[lane.idx[at]])
    }

    /// The selected PR's Productive task page, when it names one.
    fn task_link(&mut self) -> Option<String> {
        let id = self.selected_pr()?.productive_task_id.clone()?;
        if self.task_base.is_none() {
            let raw = crate::core::config::load_from(&crate::core::config::path()).raw;
            let p = &raw["productive"];
            let app = p["appUrl"].as_str().unwrap_or("https://app.productive.io");
            let org = p["orgSlug"].as_str().map(str::to_string).or_else(
                || match &p["organizationId"] {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                },
            )?;
            self.task_base = Some(format!("{}/{org}", app.trim_end_matches('/')));
        }
        let base = self.task_base.as_deref()?;
        Some(format!("{base}/tasks/{id}"))
    }

    fn step_card(&mut self, down: bool) {
        let lanes = self.lanes();
        let Some(lane) = lanes.get(self.lane) else {
            return;
        };
        let at = self.card.entry(lane.column).or_insert(0);
        *at = if down {
            (*at + 1).min(lane.idx.len().saturating_sub(1))
        } else {
            at.saturating_sub(1)
        };
    }

    fn card(p: &Pr, selected: bool, width: u16) -> (Vec<Line<'static>>, Style) {
        let (mark, color) = checks_mark(p);
        let w = width.saturating_sub(2) as usize;
        // ✎: a reviewer's latest review asks for changes.
        let blocked = if p.changes_requested_by.is_empty() {
            ""
        } else {
            "✎ "
        };
        let mut title = wrap(&format!("{mark}{blocked}{}", p.title), w);
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
        let task = p
            .productive_task_id
            .as_deref()
            .map(|id| format!(" [P-{id}]"))
            .unwrap_or_default();
        lines.push(Line::styled(
            fit(&format!("{}#{}{task}{draft}", p.repo, p.number), w),
            Style::default().fg(Color::Cyan),
        ));
        let comments = if p.comments > 0 {
            format!("  💬 {}", p.comments)
        } else {
            String::new()
        };
        let rot = rot_color(p.rotting);
        lines.push(Line::from(vec![
            Span::styled(format!("{}  ", p.size), Style::default().fg(Color::Gray)),
            Span::styled(ago(Some(p.updated_at)), Style::default().fg(rot)),
            Span::styled(comments, Style::default().fg(Color::Gray)),
        ]));
        // The border says how long it has waited; the selected card's is bold.
        let border = Style::default().fg(rot);
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
        lane: &Lane,
        selected_lane: bool,
        (more_left, more_right): (bool, bool),
    ) {
        let (col, idx) = (&lane.column, &lane.idx);
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
                &match lane.hidden_drafts {
                    0 => format!("{left} {} ({}) {right}", lane.title, idx.len()),
                    n => format!(
                        "{left} {} ({}, {n} draft hidden) {right}",
                        lane.title,
                        idx.len()
                    ),
                },
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
                .border_type(if selected {
                    BorderType::Thick
                } else {
                    BorderType::Rounded
                })
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
            self.lane = self.lane.min(self.lanes().len().saturating_sub(1));
        }
        // Keep the board fresh without a keypress while it is on screen — no-op off screen, and
        // a no-op while a fetch is in flight (`Loader::start` guards that).
        if let Some(mode) = self.pull_mode() {
            self.fetch(mode);
        }
    }

    fn refresh(&mut self) {
        self.fetch(Mode::Refresh);
    }

    fn card_stat(&self, s: &mut CardStat) {
        if let Some(v) = &self.load.data {
            let prs = &v.snap.prs;
            s.pr_total = Some(prs.len());
            s.pr_waiting = Some(
                prs.iter()
                    .filter(|p| p.column == Column::WaitingOnMe)
                    .count(),
            );
        }
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        // Only the active tab draws, so this doubles as our "on screen" heartbeat.
        self.shown_at = Some(Instant::now());
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
                " ←/→ lane · ↑/↓ card · ⏎ open · t task · c copy · D drafts · A waiting on author · r refresh ",
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
        let lanes = self.lanes();
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
                let n = self.lanes().len();
                self.lane = (self.lane + 1).min(n.saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Char('h') => self.lane = self.lane.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.step_card(true),
            KeyCode::Up | KeyCode::Char('k') => self.step_card(false),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Enter | KeyCode::Char('o') => {
                if let Some(url) = self.selected_pr().map(|p| p.url.clone()) {
                    // Opening a PR usually precedes merging or closing it out there — start a
                    // window of real refreshes so that change shows up within a minute.
                    self.watch_until = Some(Instant::now() + WATCH);
                    if let Err(e) = (self.opener)(&url) {
                        return Outcome::Log(e);
                    }
                }
            }
            KeyCode::Char('t') => {
                return match self.task_link() {
                    Some(url) => match (self.opener)(&url) {
                        Ok(()) => Outcome::None,
                        Err(e) => Outcome::Log(e),
                    },
                    None => Outcome::Log("no Productive task linked on this PR".into()),
                };
            }
            KeyCode::Char('c') => {
                if let Some(p) = self.selected_pr() {
                    return Outcome::Log(match copy(&p.url) {
                        Ok(()) => format!("copied {}", p.url),
                        Err(e) => e,
                    });
                }
            }
            KeyCode::Char('D') => {
                self.hide_drafts = !self.hide_drafts;
                self.card.clear();
            }
            KeyCode::Char('A') => {
                self.show_author = !self.show_author;
                self.lane = self.lane.min(self.lanes().len().saturating_sub(1));
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
    use ratatui::style::Color;

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
            productive_task_id: None,
            changes_requested_by: vec![],
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
        let got: Vec<(String, Vec<usize>)> = lanes(&prs, true, false)
            .into_iter()
            .map(|l| (l.title, l.idx))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Draft".to_string(), vec![1]),
                ("In review".to_string(), vec![]),
                ("Ready to merge".to_string(), vec![]),
                ("Waiting on me".to_string(), vec![0]),
                ("Mentions".to_string(), vec![]),
            ]
        );
        // `A`: waiting on author gets its lane, before Mentions.
        let titles: Vec<String> = lanes(&prs, true, true)
            .into_iter()
            .map(|l| l.title)
            .collect();
        assert_eq!(titles[4], "Waiting on author");
    }

    #[test]
    fn a_draft_asking_for_my_review_stays_hidden_until_d() {
        let mut asked = pr(Column::WaitingOnMe, 5, "not ready yet");
        asked.draft = true;
        let mut v = view_with(vec![asked, pr(Column::WaitingOnMe, 6, "ready")]);
        let s = screen(&mut v, 160, 20);
        assert!(
            s.contains("Waiting on me (1, 1 draft") && !s.contains("not ready yet"),
            "{s}"
        );
        key(&mut v, KeyCode::Char('D'));
        let s = screen(&mut v, 160, 20);
        assert!(
            s.contains("Waiting on me (2)") && s.contains("not ready yet"),
            "{s}"
        );
    }

    static OPENED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    fn record(url: &str) -> Result<(), String> {
        OPENED.lock().unwrap().push(url.to_string());
        Ok(())
    }

    #[test]
    fn cards_show_the_task_and_t_opens_it() {
        let mut linked = pr(Column::DraftMine, 8, "linked");
        linked.productive_task_id = Some("123".into());
        linked.changes_requested_by = vec!["lead".into()];
        let mut v = view_with(vec![linked]);
        v.opener = record;
        v.task_base = Some("https://app.example/42-acme".into());
        let s = screen(&mut v, 160, 20);
        assert!(s.contains("[P-123]") && s.contains("✎"), "{s}");
        key(&mut v, KeyCode::Char('t'));
        assert!(
            OPENED
                .lock()
                .unwrap()
                .contains(&"https://app.example/42-acme/tasks/123".to_string())
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
    fn cards_are_coloured_by_how_long_they_wait() {
        assert_eq!(rot_color(Rotting::Fresh), Color::Green);
        assert_eq!(rot_color(Rotting::Warming), Color::Yellow);
        assert_eq!(rot_color(Rotting::Stale), Color::Yellow);
        assert_eq!(rot_color(Rotting::Rotting), Color::Red);
        assert_eq!(rot_color(Rotting::Critical), Color::Red);
        let mut old = pr(Column::WaitingOnMe, 1, "old one");
        old.rotting = Rotting::Critical;
        let mut v = view_with(vec![old]);
        let mut t = Terminal::new(TestBackend::new(160, 20)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        let corner = buf
            .content()
            .iter()
            .find(|c| c.symbol() == "╭")
            .expect("a card");
        assert_eq!(corner.fg, Color::Red);
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

    #[test]
    fn it_pulls_periodically_only_while_on_screen() {
        // Injected data carries no fetch timestamp, so the loader always counts as stale —
        // this isolates the on-screen / mode logic from the staleness throttle.
        let mut v = view_with(vec![pr(Column::WaitingOnMe, 1, "review me")]);
        assert_eq!(v.pull_mode(), None, "never drawn ⇒ off screen ⇒ no pull");
        screen(&mut v, 160, 20); // drawing stamps the on-screen marker
        assert_eq!(
            v.pull_mode(),
            Some(Mode::Auto),
            "on screen ⇒ a cheap cached pull"
        );
        // Switched to another tab: the last draw ages out and we stop pulling.
        v.shown_at = Instant::now().checked_sub(ON_SCREEN * 2);
        assert_eq!(
            v.pull_mode(),
            None,
            "drawn too long ago ⇒ off screen ⇒ no pull"
        );
    }

    #[test]
    fn opening_a_pr_arms_a_real_refresh() {
        let mut v = view_with(vec![pr(Column::DraftMine, 8, "mine")]);
        v.opener = record;
        screen(&mut v, 160, 20); // on screen
        assert_eq!(
            v.pull_mode(),
            Some(Mode::Auto),
            "before opening a PR: a cheap pull"
        );
        key(&mut v, KeyCode::Enter);
        assert!(
            OPENED.lock().unwrap().iter().any(|u| u.contains("/pull/8")),
            "Enter opens the PR in the browser"
        );
        screen(&mut v, 160, 20); // a redraw keeps us on screen
        assert_eq!(
            v.pull_mode(),
            Some(Mode::Refresh),
            "after opening a PR: a real refresh, so a merge out there clears fast"
        );
    }

    #[test]
    fn the_refresh_window_lapses_back_to_cheap_pulls() {
        let mut v = view_with(vec![pr(Column::WaitingOnMe, 1, "review me")]);
        screen(&mut v, 160, 20); // on screen
        v.watch_until = Some(Instant::now() + WATCH);
        assert_eq!(
            v.pull_mode(),
            Some(Mode::Refresh),
            "inside the window ⇒ real refresh"
        );
        v.watch_until = Instant::now().checked_sub(Duration::from_secs(1));
        assert_eq!(
            v.pull_mode(),
            Some(Mode::Auto),
            "window lapsed ⇒ back to a cheap pull"
        );
    }

    #[test]
    fn a_fresh_pull_waits_for_the_interval() {
        let mut v = PrsView::new();
        // Settle a real (but environment-free) fetch so the loader carries a fresh timestamp.
        v.load.start(|| {
            Ok(PrView {
                snap: Snapshot {
                    user: "me".into(),
                    ..Default::default()
                },
                stale: false,
                error: None,
            })
        });
        let until = Instant::now() + Duration::from_secs(2);
        while v.load.loading && Instant::now() < until {
            v.load.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!v.load.loading, "the fetch settled");
        screen(&mut v, 160, 20); // on screen
        assert_eq!(
            v.pull_mode(),
            None,
            "just fetched ⇒ not stale yet ⇒ no second pull"
        );
    }
}
