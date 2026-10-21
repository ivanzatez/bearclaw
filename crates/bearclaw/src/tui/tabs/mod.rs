//! The dashboard's tabs: the bar on row 0, the [`View`] every non-Sessions tab implements,
//! and the pieces they share ([`loader`], [`pager`], [`input`]).

pub mod editor;
pub mod input;
pub mod loader;
pub mod notes;
pub mod pager;
pub mod prs;
pub mod session;
pub mod settings;
pub mod tasks;
pub mod todo;
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
    Claw,
    Prs,
    Tasks,
    Todo,
    Usage,
    Brain,
    Notes,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 9] = [
        Tab::Sessions,
        Tab::Claw,
        Tab::Prs,
        Tab::Tasks,
        Tab::Todo,
        Tab::Usage,
        Tab::Brain,
        Tab::Notes,
        Tab::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Sessions => "Sessions",
            Tab::Claw => "Claw",
            Tab::Prs => "PRs",
            Tab::Tasks => "Tasks",
            Tab::Todo => "Todo",
            Tab::Usage => "Usage",
            Tab::Brain => "Memory",
            Tab::Notes => "Notepad",
            Tab::Settings => "Settings",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Tab::Sessions => "Se",
            Tab::Claw => "Cl",
            Tab::Prs => "PR",
            Tab::Tasks => "Ta",
            Tab::Todo => "To",
            Tab::Usage => "Us",
            Tab::Brain => "Me",
            Tab::Notes => "No",
            Tab::Settings => "St",
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

/// The refresh button at the right end of the bar, when it fits after `used` columns of tabs
/// (with a gap): its label and first column. Shortens to the icon alone before it disappears.
pub fn refresh_button(width: u16, used: u16) -> Option<(&'static str, u16)> {
    [" ⟳ refresh ", " ⟳ "].into_iter().find_map(|label| {
        let w = width_of(label) as u16;
        (used + 1 + w <= width).then_some((label, width - w))
    })
}

pub fn hit(ranges: &[(Tab, u16, u16)], x: u16) -> Option<Tab> {
    ranges
        .iter()
        .find(|&&(_, x0, x1)| x >= x0 && x < x1)
        .map(|&(t, _, _)| t)
}

/// A key that steps tabs: `Some(true)` = next (Tab, Shift-→), `Some(false)` = previous
/// (Shift-Tab, Shift-←). Bare arrows are left to the tabs.
pub fn tab_step(k: KeyEvent) -> Option<bool> {
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    match k.code {
        KeyCode::Tab => Some(true),
        KeyCode::BackTab => Some(false),
        KeyCode::Right if shift => Some(true),
        KeyCode::Left if shift => Some(false),
        _ => None,
    }
}

/// What a tab asks the loop to do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    None,
    Quit,
    /// Back to the Sessions tab.
    Home,
    /// Open this file in the built-in editor over the dashboard.
    Edit(PathBuf),
    /// One line for the events log.
    Log(String),
    /// Show this session (by session id) on the Sessions tab, opened.
    Session(String),
    /// Start a new Claude session with this first prompt and open it.
    Spawn(String),
}

/// The Claw tab's character card reads a few numbers it doesn't own from the tabs that do.
/// Each field stays `None` until that tab's data has loaded, so the card can show a placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct CardStat {
    pub pr_total: Option<usize>,
    pub pr_waiting: Option<usize>,
    pub usage_session: Option<f64>,
    pub usage_week: Option<f64>,
}

pub trait View {
    /// The tab just became active: start a fetch if its data is stale.
    fn on_show(&mut self);
    /// Every loop iteration, active or not: take whatever background work finished.
    fn tick(&mut self);
    fn draw(&mut self, f: &mut Frame, area: Rect);
    fn key(&mut self, k: KeyEvent) -> Outcome;
    /// Fetch or re-read now, whatever the cache says (`r`, or the bar's refresh button).
    fn refresh(&mut self);
    /// Pasted text, for a view with a text input open; dropped otherwise.
    fn paste(&mut self, _text: &str) {}
    /// The session ids alive right now, after every poll.
    fn live(&mut self, _ids: &[String]) {}
    /// The sessions alive right now, after every poll.
    fn sessions(&mut self, _rows: &[crate::core::discovery::Session]) {}
    /// A left click at (`col`, `row`) of the screen, inside this tab.
    fn click(&mut self, _col: u16, _row: u16) -> Outcome {
        Outcome::None
    }
    /// A text input is open: every key is its, Tab and `q` included.
    fn typing(&self) -> bool {
        false
    }
    /// Contribute this tab's cached numbers to the Claw character card, when it has any.
    fn card_stat(&self, _s: &mut CardStat) {}
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
    /// Every tab but Sessions and Claw (the dashboard and a terminal, drawn by the loop).
    views: Vec<(Tab, Box<dyn View>)>,
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

impl Tabs {
    pub fn new() -> Self {
        let views: Vec<(Tab, Box<dyn View>)> = Tab::ALL
            .iter()
            .filter_map(|&t| {
                let v: Box<dyn View> = match t {
                    Tab::Sessions | Tab::Claw => return None,
                    Tab::Prs => Box::new(prs::PrsView::new()),
                    Tab::Tasks => Box::new(tasks::TasksView::new()),
                    Tab::Todo => Box::new(todo::TodoView::new()),
                    Tab::Usage => Box::new(usage::UsageView::new()),
                    Tab::Brain => Box::new(notes::NotesView::brain()),
                    Tab::Notes => Box::new(notes::NotesView::notepad()),
                    Tab::Settings => Box::new(settings::SettingsView::new()),
                };
                Some((t, v))
            })
            .collect();
        Tabs {
            active: Tab::Sessions,
            views,
        }
    }

    /// `None` for Sessions and Claw: those the loop draws itself.
    pub fn view(&mut self, t: Tab) -> Option<&mut (dyn View + 'static)> {
        self.views
            .iter_mut()
            .find(|(tab, _)| *tab == t)
            .map(|(_, v)| v.as_mut())
    }

    pub fn switch(&mut self, t: Tab) {
        self.active = t;
        if let Some(v) = self.view(t) {
            v.on_show();
        }
    }

    pub fn tick_all(&mut self) {
        for (_, v) in &mut self.views {
            v.tick();
        }
    }

    pub fn live(&mut self, rows: &[crate::core::discovery::Session]) {
        let ids: Vec<String> = rows.iter().filter_map(|r| r.session_id.clone()).collect();
        for (_, v) in &mut self.views {
            v.live(&ids);
            v.sessions(rows);
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
    fn claw_is_the_second_tab_and_not_a_view() {
        assert_eq!(Tab::Sessions.next(), Tab::Claw);
        let mut tabs = Tabs::new();
        assert!(tabs.view(Tab::Claw).is_none() && tabs.view(Tab::Sessions).is_none());
        assert!(tabs.view(Tab::Prs).is_some() && tabs.view(Tab::Settings).is_some());
    }

    #[test]
    fn tab_cycle_wraps() {
        assert_eq!(Tab::Settings.next(), Tab::Sessions);
        assert_eq!(Tab::Sessions.prev(), Tab::Settings);
        assert_eq!(Tab::Claw.next(), Tab::Prs);
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
            s.contains("‹") && s.contains("Memory") && s.contains("›"),
            "{s}"
        );
        assert_eq!(hit(&hits, 0), Some(Tab::Usage));
        assert_eq!(hit(&hits, width_of(&s) as u16 - 1), Some(Tab::Notes));

        let (none, hits) = tab_bar(Tab::Sessions, 0, None);
        assert_eq!(width_of(&text(&none)), 0);
        assert_eq!(hit(&hits, 0), None);
    }

    #[test]
    fn tab_and_shift_arrows_step_tabs() {
        let key = |c, m| KeyEvent::new(c, m);
        assert_eq!(tab_step(key(KeyCode::Tab, KeyModifiers::NONE)), Some(true));
        assert_eq!(
            tab_step(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(false)
        );
        assert_eq!(
            tab_step(key(KeyCode::Right, KeyModifiers::SHIFT)),
            Some(true)
        );
        assert_eq!(
            tab_step(key(KeyCode::Left, KeyModifiers::SHIFT)),
            Some(false)
        );
        // Bare arrows stay the tabs' own (→ opens a session, ← goes back).
        assert_eq!(tab_step(key(KeyCode::Right, KeyModifiers::NONE)), None);
        assert_eq!(tab_step(key(KeyCode::Left, KeyModifiers::NONE)), None);
    }

    #[test]
    fn the_refresh_button_sits_right_when_it_fits() {
        assert_eq!(refresh_button(120, 55), Some((" ⟳ refresh ", 109)));
        assert_eq!(refresh_button(60, 55), Some((" ⟳ ", 57)));
        assert_eq!(refresh_button(56, 55), None);
    }

    #[test]
    fn the_sessions_label_carries_its_badge() {
        let (l, _) = tab_bar(Tab::Prs, 120, Some("⏸2".into()));
        assert!(text(&l).contains("Sessions ⏸2"), "{}", text(&l));
    }
}
