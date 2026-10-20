//! The Usage tab: the Claude subscription's limits as gauges.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, LineGauge, Paragraph, Wrap};

use super::loader::Loader;
use super::{CardStat, Outcome, View, common_key};
use crate::cli::render::fit;
use crate::cli::usage::resets;
use crate::core::discovery::claude_home;
use crate::core::usage::{self, Usage};

/// The core caches for 60 s itself; this only bounds how often the tab asks it.
const TTL: Duration = Duration::from_secs(60);

pub struct UsageView {
    pub(crate) load: Loader<Usage>,
}

impl Default for UsageView {
    fn default() -> Self {
        Self::new()
    }
}

fn color_for(percent: f64) -> Color {
    match percent {
        p if p >= 90.0 => Color::Red,
        p if p >= 70.0 => Color::Yellow,
        _ => Color::Green,
    }
}

impl UsageView {
    pub fn new() -> Self {
        UsageView {
            load: Loader::new(),
        }
    }

    fn fetch(&mut self, refresh: bool) {
        self.load.start(move || usage::get(&claude_home(), refresh));
    }
}

impl View for UsageView {
    fn on_show(&mut self) {
        if self.load.stale(TTL) {
            self.fetch(false);
        }
    }

    fn tick(&mut self) {
        self.load.poll();
    }

    fn refresh(&mut self) {
        self.fetch(true);
    }

    fn card_stat(&self, s: &mut CardStat) {
        if let Some(u) = &self.load.data {
            s.usage_session = u
                .limits
                .iter()
                .find(|l| l.group == "session")
                .map(|l| l.percent);
            s.usage_week = u
                .limits
                .iter()
                .find(|l| l.kind == "weekly_all")
                .map(|l| l.percent);
        }
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        let mut title = String::from(" Usage ");
        if let Some(u) = &self.load.data {
            if let Some(plan) = u.account.as_ref().and_then(|a| a.plan_label.as_deref()) {
                title.push_str(&format!("· {plan} "));
            }
            if u.stale {
                title.push_str(&format!(
                    "· stale{} ",
                    u.error
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
                " r refresh · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let Some(u) = &self.load.data else {
            let msg = match (&self.load.error, self.load.loading) {
                (Some(e), _) => e.clone(),
                (None, true) => "reading your limits…".into(),
                (None, false) => "r to read your limits".into(),
            };
            f.render_widget(
                Paragraph::new(format!(" {msg}")).wrap(Wrap { trim: false }),
                inner,
            );
            return;
        };
        let now = chrono::Utc::now();
        let bottom = inner.y + inner.height;
        let mut y = inner.y;
        let row = |y: u16| Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1);
        let mut group = "";
        for l in &u.limits {
            if l.group != group {
                if y >= bottom {
                    break;
                }
                group = &l.group;
                let name = if group == "session" {
                    "Session"
                } else {
                    "Weekly"
                };
                f.render_widget(
                    Paragraph::new(Line::styled(
                        name,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    row(y),
                );
                y += 1;
            }
            if y + 1 >= bottom {
                break;
            }
            let reset = l
                .resets_at
                .as_deref()
                .and_then(|t| resets(t, now))
                .unwrap_or_default();
            let color = color_for(l.percent);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(format!("{}  ", l.label)),
                    Span::styled(
                        format!("{:.0}%", l.percent),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  {reset}"), Style::default().fg(Color::DarkGray)),
                ])),
                row(y),
            );
            f.render_widget(
                LineGauge::default()
                    .ratio((l.percent / 100.0).clamp(0.0, 1.0))
                    .label("")
                    .filled_style(Style::default().fg(color))
                    .unfilled_style(Style::default().fg(Color::DarkGray)),
                row(y + 1),
            );
            y += 3;
        }
        if let Some(x) = u.extra_usage.as_ref().filter(|x| x.enabled)
            && y < bottom
        {
            let cur = x.currency.as_deref().unwrap_or("");
            let used = x.used.map(|v| format!("{v:.2}")).unwrap_or("?".into());
            let limit = x.limit.map(|v| format!(" / {v:.2}")).unwrap_or_default();
            f.render_widget(
                Paragraph::new(format!("Extra usage  {used}{limit} {cur}")),
                row(y),
            );
        }
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if let Some(o) = common_key(k) {
            return o;
        }
        if k.code == KeyCode::Char('r') {
            self.refresh();
        }
        Outcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::usage::{Limit, Usage};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn limit(label: &str, group: &str, percent: f64, severity: &str) -> Limit {
        Limit {
            kind: group.into(),
            group: group.into(),
            label: label.into(),
            model: None,
            percent,
            severity: severity.into(),
            resets_at: Some((chrono::Utc::now() + chrono::Duration::minutes(45)).to_rfc3339()),
            active: false,
        }
    }

    fn screen(v: &mut UsageView, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn usage_draws_a_gauge_per_limit() {
        let mut v = UsageView::new();
        v.load.data = Some(Usage {
            account: None,
            limits: vec![
                limit("Current session", "session", 42.0, "normal"),
                limit("Weekly · all models", "weekly", 87.0, "warning"),
            ],
            extra_usage: None,
            fetched_at: chrono::Utc::now().to_rfc3339(),
            stale: true,
            error: Some("timeout".into()),
        });
        let s = screen(&mut v, 80, 12);
        assert!(s.contains("Current session") && s.contains("42%"), "{s}");
        assert!(
            s.contains("Weekly · all models") && s.contains("87%"),
            "{s}"
        );
        assert!(s.contains("resets in 4"), "{s}");
        assert!(s.contains("stale"), "{s}");
    }
}
