//! The dashboard's tabs: the bar on row 0, the [`View`] every non-Sessions tab implements,
//! and the pieces they share ([`loader`], [`pager`], [`input`]).

pub mod brain;
pub mod input;
pub mod loader;
pub mod memos;
pub mod pager;
pub mod prs;
pub mod settings;
pub mod tasks;
pub mod usage;

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::cli::render::width_of;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sessions,
    Prs,
    Tasks,
    Usage,
    Brain,
    Memos,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Sessions,
        Tab::Prs,
        Tab::Tasks,
        Tab::Usage,
        Tab::Brain,
        Tab::Memos,
        Tab::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Sessions => "Sessions",
            Tab::Prs => "PRs",
            Tab::Tasks => "Tasks",
            Tab::Usage => "Usage",
            Tab::Brain => "Brain",
            Tab::Memos => "Memos",
            Tab::Settings => "Settings",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Tab::Sessions => "Ses",
            Tab::Prs => "PR",
            Tab::Tasks => "Tsk",
            Tab::Usage => "Use",
            Tab::Brain => "Brn",
            Tab::Memos => "Mem",
            Tab::Settings => "Set",
        }
    }

    fn index(self) -> usize {
        Tab::ALL.iter().position(|&t| t == self).unwrap_or(0)
    }

    pub fn next(self) -> Tab {
        Tab::ALL[(self.index() + 1) % Tab::ALL.len()]
    }

    pub fn prev(self) -> Tab {
        Tab::ALL[(self.index() + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }
}

/// The bar and each tab's `[x0, x1)` columns, for clicks. Full titles when they fit, then
/// short ones, then only the active tab between `‹ ›` arrows that step to its neighbours.
pub fn tab_bar(
    active: Tab,
    width: u16,
    badge: Option<String>,
) -> (Line<'static>, Vec<(Tab, u16, u16)>) {
    let width = width as usize;
    let on = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let off = Style::default().fg(Color::Gray);
    let label = |t: Tab, name: &str| match (&badge, t) {
        (Some(b), Tab::Sessions) => format!(" {name} {b} "),
        _ => format!(" {name} "),
    };
    for names in [Tab::title as fn(Tab) -> &'static str, Tab::short] {
        let cells: Vec<(Tab, String)> = Tab::ALL.iter().map(|&t| (t, label(t, names(t)))).collect();
        if cells.iter().map(|(_, s)| width_of(s)).sum::<usize>() > width {
            continue;
        }
        let mut spans = Vec::new();
        let mut hits = Vec::new();
        let mut x = 0u16;
        for (t, s) in cells {
            let w = width_of(&s) as u16;
            hits.push((t, x, x + w));
            x += w;
            spans.push(Span::styled(s, if t == active { on } else { off }));
        }
        return (Line::from(spans), hits);
    }
    for name in [active.title(), active.short()] {
        let mid = label(active, name);
        let (l, r) = ("‹", "›");
        if width_of(&mid) + 2 > width {
            continue;
        }
        let w = width_of(&mid) as u16;
        let hits = vec![
            (active.prev(), 0, 1),
            (active, 1, 1 + w),
            (active.next(), 1 + w, 2 + w),
        ];
        let spans = vec![
            Span::styled(l, off),
            Span::styled(mid, on),
            Span::styled(r, off),
        ];
        return (Line::from(spans), hits);
    }
    (Line::default(), Vec::new())
}

pub fn hit(ranges: &[(Tab, u16, u16)], x: u16) -> Option<Tab> {
    ranges
        .iter()
        .find(|&&(_, x0, x1)| x >= x0 && x < x1)
        .map(|&(t, _, _)| t)
}

/// What a tab asks the loop to do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    None,
    Quit,
    /// Back to the Sessions tab.
    Home,
    /// Suspend the dashboard and open this file in `$VISUAL` / `$EDITOR`.
    Edit(PathBuf),
    /// One line for the events log.
    Log(String),
}

pub trait View {
    /// The tab just became active: start a fetch if its data is stale.
    fn on_show(&mut self);
    /// Every loop iteration, active or not: take whatever background work finished.
    fn tick(&mut self);
    fn draw(&mut self, f: &mut Frame, area: Rect);
    fn key(&mut self, k: KeyEvent) -> Outcome;
    /// A text input is open: every key is its, Tab and `q` included.
    fn typing(&self) -> bool {
        false
    }
}

/// The keys every tab shares at its top level (no input, no pager open).
pub fn common_key(k: KeyEvent) -> Option<Outcome> {
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        return (k.code == KeyCode::Char('c')).then_some(Outcome::Quit);
    }
    match k.code {
        KeyCode::Char('q') => Some(Outcome::Quit),
        KeyCode::Esc => Some(Outcome::Home),
        _ => None,
    }
}

/// The non-Sessions tabs' views, in [`Tab::ALL`] order, and which tab is up.
pub struct Tabs {
    pub active: Tab,
    views: Vec<Box<dyn View>>,
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

impl Tabs {
    pub fn new() -> Self {
        let views: Vec<Box<dyn View>> = Tab::ALL[1..]
            .iter()
            .map(|&t| match t {
                Tab::Prs => Box::new(prs::PrsView::new()) as Box<dyn View>,
                Tab::Tasks => Box::new(tasks::TasksView::new()),
                Tab::Usage => Box::new(usage::UsageView::new()),
                Tab::Brain => Box::new(brain::BrainView::new()),
                Tab::Memos => Box::new(memos::MemosView::new()),
                Tab::Settings => Box::new(settings::SettingsView::new()),
                Tab::Sessions => unreachable!("Sessions is the dashboard itself"),
            })
            .collect();
        Tabs {
            active: Tab::Sessions,
            views,
        }
    }

    /// `None` for Sessions: that one is the dashboard itself.
    pub fn view(&mut self, t: Tab) -> Option<&mut (dyn View + 'static)> {
        let i = t.index().checked_sub(1)?;
        self.views.get_mut(i).map(|v| v.as_mut())
    }

    pub fn switch(&mut self, t: Tab) {
        self.active = t;
        if let Some(v) = self.view(t) {
            v.on_show();
        }
    }

    pub fn tick_all(&mut self) {
        for v in &mut self.views {
            v.tick();
        }
    }

    /// The active view is reading text: tab switching and quitting wait.
    pub fn typing(&mut self) -> bool {
        let t = self.active;
        self.view(t).is_some_and(|v| v.typing())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn tab_cycle_wraps() {
        assert_eq!(Tab::Settings.next(), Tab::Sessions);
        assert_eq!(Tab::Sessions.prev(), Tab::Settings);
        assert_eq!(Tab::Sessions.next(), Tab::Prs);
    }

    #[test]
    fn tab_bar_degrades_with_width() {
        let (wide, hits) = tab_bar(Tab::Sessions, 120, None);
        let w = text(&wide);
        for t in Tab::ALL {
            assert!(w.contains(t.title()), "{w}");
        }
        let &(_, x0, x1) = hits.iter().find(|h| h.0 == Tab::Prs).unwrap();
        assert_eq!(hit(&hits, (x0 + x1) / 2), Some(Tab::Prs));

        let (mid, _) = tab_bar(Tab::Prs, 40, None);
        let m = text(&mid);
        assert!(width_of(&m) <= 40, "{m}");
        for t in Tab::ALL {
            assert!(m.contains(t.short()), "{m}");
        }

        // A phone: only the active tab, with arrows that step to its neighbours.
        let (tiny, hits) = tab_bar(Tab::Brain, 14, None);
        let s = text(&tiny);
        assert!(width_of(&s) <= 14, "{s}");
        assert!(
            s.contains("‹") && s.contains("Brain") && s.contains("›"),
            "{s}"
        );
        assert_eq!(hit(&hits, 0), Some(Tab::Usage));
        assert_eq!(hit(&hits, width_of(&s) as u16 - 1), Some(Tab::Memos));

        let (none, hits) = tab_bar(Tab::Sessions, 0, None);
        assert_eq!(width_of(&text(&none)), 0);
        assert_eq!(hit(&hits, 0), None);
    }

    #[test]
    fn the_sessions_label_carries_its_badge() {
        let (l, _) = tab_bar(Tab::Prs, 120, Some("⏸2".into()));
        assert!(text(&l).contains("Sessions ⏸2"), "{}", text(&l));
    }
}
