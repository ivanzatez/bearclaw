//! The Brain tab: browse, search, read and write the brain's notes.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::input::{InputResult, LineInput};
use super::pager::Pager;
use super::{Outcome, View, common_key};
use crate::cli::render::{ago, column, fit, width_of};
use crate::core::brain::{self, Note};
use crate::core::tools::tildify;

enum Asking {
    Search,
    NewNote,
}

pub struct BrainView {
    /// `None` = follow `brain.root`, re-read on every show.
    fixed_root: Option<PathBuf>,
    root: PathBuf,
    notes: Vec<Note>,
    /// Indices into `notes` that match `query`.
    shown: Vec<usize>,
    query: String,
    input: Option<(Asking, LineInput)>,
    sel: usize,
    list: ListState,
    pager: Option<Pager>,
}

impl Default for BrainView {
    fn default() -> Self {
        Self::new()
    }
}

impl BrainView {
    pub fn new() -> Self {
        Self::with_root(None)
    }

    /// A brain at a fixed folder, whatever the config says.
    pub fn at(root: PathBuf) -> Self {
        Self::with_root(Some(root))
    }

    fn with_root(fixed_root: Option<PathBuf>) -> Self {
        BrainView {
            root: fixed_root.clone().unwrap_or_default(),
            fixed_root,
            notes: Vec::new(),
            shown: Vec::new(),
            query: String::new(),
            input: None,
            sel: 0,
            list: ListState::default(),
            pager: None,
        }
    }

    fn relist(&mut self) {
        self.root = self.fixed_root.clone().unwrap_or_else(brain::root);
        self.notes = brain::list(&self.root);
        self.refilter();
    }

    fn refilter(&mut self) {
        self.shown = brain::search(&self.notes, &self.query);
        self.sel = self.sel.min(self.shown.len().saturating_sub(1));
    }

    fn selected(&self) -> Option<&Note> {
        self.shown.get(self.sel).map(|&i| &self.notes[i])
    }

    fn input_key(&mut self, k: KeyEvent) -> Outcome {
        let Some((asking, input)) = self.input.as_mut() else {
            return Outcome::None;
        };
        let result = input.key(k);
        let searching = matches!(asking, Asking::Search);
        match result {
            InputResult::Editing => {
                if searching {
                    self.query = input.buf.clone();
                    self.sel = 0;
                    self.refilter();
                }
                Outcome::None
            }
            InputResult::Quit => Outcome::Quit,
            InputResult::Cancel => {
                self.input = None;
                if searching {
                    self.query.clear();
                    self.refilter();
                }
                Outcome::None
            }
            InputResult::Submit(text) => {
                self.input = None;
                if searching || text.trim().is_empty() {
                    return Outcome::None;
                }
                match brain::create(&self.root, &text) {
                    Ok(path) => {
                        self.relist();
                        Outcome::Edit(path)
                    }
                    Err(e) => Outcome::Log(format!("new note: {e}")),
                }
            }
        }
    }
}

impl View for BrainView {
    fn on_show(&mut self) {
        self.relist();
    }

    fn tick(&mut self) {}

    fn typing(&self) -> bool {
        self.input.is_some()
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        if let Some(p) = self.pager.as_mut() {
            p.draw(f, area);
            return;
        }
        let mut title = format!(
            " Brain · {} · {} notes ",
            tildify(&self.root.display().to_string()),
            self.notes.len()
        );
        if !self.query.is_empty() {
            title.push_str(&format!("· /{} → {} ", self.query, self.shown.len()));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(fit(&title, area.width.saturating_sub(2) as usize))
            .title_bottom(Line::styled(
                " ⏎ read · e edit · n new · / search · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [body, prompt] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::from(self.input.is_some())),
        ])
        .areas(inner);
        if let Some((_, input)) = &self.input {
            input.draw(f, prompt);
        }
        if self.notes.is_empty() {
            let root = tildify(&self.root.display().to_string());
            f.render_widget(
                Paragraph::new(format!(" no notes yet in {root}\n n to write one"))
                    .wrap(Wrap { trim: false }),
                body,
            );
            return;
        }
        let width = body.width as usize;
        let items: Vec<ListItem> = self
            .shown
            .iter()
            .map(|&i| {
                let n = &self.notes[i];
                let age = format!(" {} ", column(&ago(Some(n.mtime_ms)), 4));
                let rel = format!(" {}", fit(&n.rel, 30));
                let title_w = width.saturating_sub(1 + width_of(&rel) + width_of(&age));
                ListItem::new(Line::from(vec![
                    Span::raw(format!(" {}", column(&n.title, title_w))),
                    Span::styled(rel, Style::default().fg(Color::DarkGray)),
                    Span::styled(age, Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect();
        self.list.select(Some(self.sel));
        let list = List::new(items).highlight_style(Style::default().bg(Color::DarkGray));
        f.render_stateful_widget(list, body, &mut self.list);
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if self.input.is_some() {
            return self.input_key(k);
        }
        if let Some(p) = self.pager.as_mut() {
            if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
                return Outcome::Quit;
            }
            if !p.key(k) {
                self.pager = None;
            }
            return Outcome::None;
        }
        // Esc clears a filter before it leaves the tab.
        if k.code == KeyCode::Esc && !self.query.is_empty() {
            self.query.clear();
            self.refilter();
            return Outcome::None;
        }
        if let Some(o) = common_key(k) {
            return o;
        }
        match k.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.sel = (self.sel + 1).min(self.shown.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Char('r') => self.relist(),
            KeyCode::Char('/') => {
                let mut input = LineInput::new("search");
                input.buf = self.query.clone();
                self.input = Some((Asking::Search, input));
            }
            KeyCode::Char('n') => {
                self.input = Some((Asking::NewNote, LineInput::new("new note title")))
            }
            KeyCode::Char('e') => {
                if let Some(n) = self.selected() {
                    return Outcome::Edit(n.path.clone());
                }
            }
            KeyCode::Enter => {
                if let Some(n) = self.selected() {
                    let text =
                        std::fs::read_to_string(&n.path).unwrap_or_else(|e| format!("({e})"));
                    self.pager = Some(Pager::new(n.title.clone(), text));
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

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn screen(v: &mut BrainView, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn brain_tab_filters_while_typing() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("api.md"), "# Deploy api\nuses helm").unwrap();
        std::fs::write(d.path().join("web.md"), "# Deploy web\nuses vercel").unwrap();
        let mut v = BrainView::at(d.path().to_path_buf());
        v.on_show();
        let all = screen(&mut v, 80, 10);
        assert!(
            all.contains("Deploy api") && all.contains("Deploy web"),
            "{all}"
        );

        assert_eq!(v.key(k(KeyCode::Char('/'))), Outcome::None);
        assert!(v.typing());
        for c in "helm".chars() {
            v.key(k(KeyCode::Char(c)));
        }
        let found = screen(&mut v, 80, 10);
        assert!(
            found.contains("Deploy api") && !found.contains("Deploy web"),
            "{found}"
        );
        // `q` is text while the query is open.
        assert_eq!(v.key(k(KeyCode::Char('q'))), Outcome::None);
        assert!(v.typing());
        v.key(k(KeyCode::Esc));
        assert!(!v.typing());

        assert_eq!(v.key(k(KeyCode::Char('n'))), Outcome::None);
        assert!(v.typing());
        for c in "Release steps".chars() {
            v.key(k(KeyCode::Char(c)));
        }
        let made = d.path().join("release-steps.md");
        assert_eq!(v.key(k(KeyCode::Enter)), Outcome::Edit(made.clone()));
        assert!(made.exists());
    }

    #[test]
    fn an_empty_brain_says_how_to_start() {
        let d = tempfile::tempdir().unwrap();
        let mut v = BrainView::at(d.path().join("nothing-here"));
        v.on_show();
        let s = screen(&mut v, 90, 8);
        assert!(
            s.contains("no notes yet") && s.contains("n to write one"),
            "{s}"
        );
        assert_eq!(v.key(k(KeyCode::Char('e'))), Outcome::None);
        assert_eq!(v.key(k(KeyCode::Enter)), Outcome::None);
    }
}
