//! The PRs tab: `bearclaw prs` as a list grouped by column.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::loader::Loader;
use super::{Outcome, View, common_key};
use crate::cli::render::{ago, column, fit, width_of};
use crate::core::prs::{self, Column, Mode, Pr, View as PrView};
use crate::core::tools::open_url;

/// `prs.refresh` decides when `Mode::Auto` really fetches; this only bounds how often we ask.
const TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Header(Column, usize),
    /// Index into the snapshot's `prs`.
    Pr(usize),
}

/// Headers and PRs in [`Column::ALL`] order; empty columns are left out.
pub fn flatten(prs: &[Pr]) -> Vec<Item> {
    let mut out = Vec::new();
    for col in Column::ALL {
        let idx: Vec<usize> = (0..prs.len()).filter(|&i| prs[i].column == col).collect();
        if idx.is_empty() {
            continue;
        }
        out.push(Item::Header(col, idx.len()));
        out.extend(idx.into_iter().map(Item::Pr));
    }
    out
}

pub struct PrsView {
    pub(crate) load: Loader<PrView>,
    /// Position among the PR rows (headers don't count).
    sel: usize,
    list: ListState,
}

impl Default for PrsView {
    fn default() -> Self {
        Self::new()
    }
}

impl PrsView {
    pub fn new() -> Self {
        PrsView {
            load: Loader::new(),
            sel: 0,
            list: ListState::default(),
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
        let prs = self.prs();
        flatten(prs)
            .into_iter()
            .filter_map(|i| match i {
                Item::Pr(n) => Some(n),
                Item::Header(..) => None,
            })
            .nth(self.sel)
            .map(|n| &prs[n])
    }

    fn pr_count(&self) -> usize {
        self.prs().len()
    }

    fn row(p: &Pr, width: usize) -> Line<'static> {
        let id = format!(" {}#{} ", p.repo, p.number);
        let (mark, color) = match p.checks.as_deref() {
            Some("success") => ("✓", Color::Green),
            Some("failure" | "error") => ("✗", Color::Red),
            Some(_) => ("●", Color::Yellow),
            None => (" ", Color::Gray),
        };
        let meta = format!(" @{} {} ", p.author, column(&p.size, 2));
        let age = format!(" {} ", column(&ago(Some(p.updated_at)), 4));
        let title_w = width.saturating_sub(width_of(&id) + width_of(&meta) + 1 + width_of(&age));
        let draft = if p.draft { "draft · " } else { "" };
        Line::from(vec![
            Span::styled(id, Style::default().fg(Color::Cyan)),
            Span::raw(column(&format!("{draft}{}", p.title), title_w)),
            Span::styled(meta, Style::default().fg(Color::Gray)),
            Span::styled(mark, Style::default().fg(color)),
            Span::styled(age, Style::default().fg(Color::DarkGray)),
        ])
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
            self.sel = self.sel.min(self.pr_count().saturating_sub(1));
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
                " ⏎/o open · r refresh · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
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
        let prs = self.prs();
        if prs.is_empty() {
            f.render_widget(Paragraph::new(" nothing needs you 🎉"), inner);
            return;
        }
        let width = inner.width as usize;
        let mut selected_row = 0;
        let mut seen = 0;
        let items: Vec<ListItem> = flatten(prs)
            .into_iter()
            .enumerate()
            .map(|(row, item)| match item {
                Item::Header(col, n) => ListItem::new(Line::styled(
                    format!(" {} · {n}", col.title()),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Item::Pr(i) => {
                    if seen == self.sel {
                        selected_row = row;
                    }
                    seen += 1;
                    ListItem::new(Self::row(&prs[i], width))
                }
            })
            .collect();
        self.list.select(Some(selected_row));
        let list = List::new(items).highlight_style(Style::default().bg(Color::DarkGray));
        f.render_stateful_widget(list, inner, &mut self.list);
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if let Some(o) = common_key(k) {
            return o;
        }
        match k.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.sel = (self.sel + 1).min(self.pr_count().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
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

    #[test]
    fn rows_group_by_column() {
        let prs = vec![
            pr(Column::DraftMine, 2, "draft thing"),
            pr(Column::WaitingOnMe, 1, "review me"),
        ];
        let items = flatten(&prs);
        // Column::ALL order: waiting on me comes before my drafts.
        assert_eq!(
            items,
            vec![
                Item::Header(Column::WaitingOnMe, 1),
                Item::Pr(1),
                Item::Header(Column::DraftMine, 1),
                Item::Pr(0),
            ]
        );
        let mut v = view_with(prs);
        assert_eq!(v.selected_pr().map(|p| p.number), Some(1));
        v.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(v.selected_pr().map(|p| p.number), Some(2));
        v.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(v.selected_pr().map(|p| p.number), Some(2));
        let s = screen(&mut v, 100, 20);
        assert!(
            s.contains("Waiting on me") && s.contains("api#1") && s.contains("review me"),
            "{s}"
        );
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
