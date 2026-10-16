//! The Brain and Notepad tabs: a folder of notes to browse, filter, create, edit and delete.
//! Brain holds `.md` notes every Claude session shares; Notepad is your own `.md`/`.txt`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::input::{InputResult, LineInput};
use super::{Outcome, View, common_key};
use crate::cli::render::{ago, column, fit, width_of};
use crate::core::brain::{self, Note};
use crate::core::notes;
use crate::core::tools::tildify;

enum Asking {
    Search,
    NewNote,
}

pub struct NotesView {
    name: &'static str,
    exts: &'static [&'static str],
    /// Where the folder is when not fixed: re-read on every show (config edits apply).
    resolve: fn() -> PathBuf,
    fixed_root: Option<PathBuf>,
    root: PathBuf,
    notes: Vec<Note>,
    /// Indices into `notes` that match `query`.
    shown: Vec<usize>,
    query: String,
    input: Option<(Asking, LineInput)>,
    /// A note waiting for `y` to be deleted.
    confirm_delete: Option<usize>,
    sel: usize,
    list: ListState,
}

impl NotesView {
    pub fn brain() -> Self {
        Self::new("Brain", &["md"], brain::root, None)
    }

    pub fn notepad() -> Self {
        Self::new("Notepad", &["md", "txt"], notes::root, None)
    }

    /// The brain at a fixed folder, whatever the config says.
    pub fn brain_at(root: PathBuf) -> Self {
        Self::new("Brain", &["md"], brain::root, Some(root))
    }

    pub fn notepad_at(root: PathBuf) -> Self {
        Self::new("Notepad", &["md", "txt"], notes::root, Some(root))
    }

    fn new(
        name: &'static str,
        exts: &'static [&'static str],
        resolve: fn() -> PathBuf,
        fixed_root: Option<PathBuf>,
    ) -> Self {
        NotesView {
            name,
            exts,
            resolve,
            root: fixed_root.clone().unwrap_or_default(),
            fixed_root,
            notes: Vec::new(),
            shown: Vec::new(),
            query: String::new(),
            input: None,
            confirm_delete: None,
            sel: 0,
            list: ListState::default(),
        }
    }

    fn relist(&mut self) {
        self.root = self.fixed_root.clone().unwrap_or_else(self.resolve);
        self.notes = brain::list_with(&self.root, self.exts);
        self.refilter();
    }

    fn refilter(&mut self) {
        self.shown = brain::search(&self.notes, &self.query);
        self.sel = self.sel.min(self.shown.len().saturating_sub(1));
    }

    fn selected(&self) -> Option<&Note> {
        self.shown.get(self.sel).map(|&i| &self.notes[i])
    }

    /// A new note: markdown, or plain text when the Notepad title ends in `.txt`.
    fn create(&mut self, title: &str) -> Outcome {
        let title = title.trim();
        let (title, ext) = match title.strip_suffix(".txt") {
            Some(t) if self.exts.contains(&"txt") => (t, "txt"),
            _ => (title, "md"),
        };
        match brain::create_ext(&self.root, title, ext) {
            Ok(path) => {
                self.relist();
                if let Some(i) = self.shown.iter().position(|&i| self.notes[i].path == path) {
                    self.sel = i;
                }
                Outcome::Edit(path)
            }
            Err(e) => Outcome::Log(format!("new note: {e}")),
        }
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
                self.create(&text)
            }
        }
    }
}

impl View for NotesView {
    fn on_show(&mut self) {
        self.relist();
    }

    fn tick(&mut self) {}

    fn refresh(&mut self) {
        self.relist();
    }

    fn typing(&self) -> bool {
        self.input.is_some() || self.confirm_delete.is_some()
    }

    fn paste(&mut self, text: &str) {
        let Some((asking, input)) = self.input.as_mut() else {
            return;
        };
        input.paste(text);
        if matches!(asking, Asking::Search) {
            self.query = input.buf.clone();
            self.sel = 0;
            self.refilter();
        }
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        let mut title = format!(
            " {} · {} · {} notes ",
            self.name,
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
                " ⏎/e edit · n new · / search · x delete · esc back ",
                Style::default().fg(Color::DarkGray),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let asking = self.input.is_some() || self.confirm_delete.is_some();
        let [body, prompt] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(u16::from(asking))])
                .areas(inner);
        if let Some((_, input)) = &self.input {
            input.draw(f, prompt);
        }
        if let Some(i) = self.confirm_delete {
            f.render_widget(
                Paragraph::new(Line::styled(
                    format!(" delete {}? y/n", self.notes[i].rel),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                prompt,
            );
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
        if self.shown.is_empty() {
            f.render_widget(
                Paragraph::new(format!(" nothing matches /{}", self.query)),
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
        if let Some(i) = self.confirm_delete.take() {
            if k.code == KeyCode::Char('y') {
                let path = self.notes[i].path.clone();
                if let Err(e) = std::fs::remove_file(&path) {
                    return Outcome::Log(format!("delete: {e}"));
                }
                self.relist();
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
                let hint = if self.exts.contains(&"txt") {
                    "new note title (end in .txt for plain text)"
                } else {
                    "new note title"
                };
                self.input = Some((Asking::NewNote, LineInput::new(hint)));
            }
            KeyCode::Char('x') => self.confirm_delete = self.shown.get(self.sel).copied(),
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(n) = self.selected() {
                    return Outcome::Edit(n.path.clone());
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

    fn screen(v: &mut NotesView, w: u16, h: u16) -> String {
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
        let mut v = NotesView::brain_at(d.path().to_path_buf());
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
    fn notepad_new_note_opens_the_editor() {
        let d = tempfile::tempdir().unwrap();
        let mut v = NotesView::notepad_at(d.path().to_path_buf());
        v.on_show();
        v.key(k(KeyCode::Char('n')));
        for c in "Ideas.txt".chars() {
            v.key(k(KeyCode::Char(c)));
        }
        let made = d.path().join("ideas.txt");
        assert_eq!(v.key(k(KeyCode::Enter)), Outcome::Edit(made.clone()));
        assert_eq!(std::fs::read_to_string(&made).unwrap(), "Ideas\n\n");
        // Enter on a note opens it in the editor too.
        assert_eq!(v.key(k(KeyCode::Enter)), Outcome::Edit(made));
    }

    #[test]
    fn notepad_delete_needs_confirmation() {
        let d = tempfile::tempdir().unwrap();
        let note = d.path().join("todo.txt");
        std::fs::write(&note, "todo\n").unwrap();
        let mut v = NotesView::notepad_at(d.path().to_path_buf());
        v.on_show();
        v.key(k(KeyCode::Char('x')));
        assert!(v.typing());
        v.key(k(KeyCode::Char('n')));
        assert!(!v.typing() && note.exists());
        v.key(k(KeyCode::Char('x')));
        v.key(k(KeyCode::Char('y')));
        assert!(!note.exists());
        let s = screen(&mut v, 80, 8);
        assert!(s.contains("no notes yet"), "{s}");
    }

    #[test]
    fn a_paste_goes_into_the_open_prompt() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("api.md"), "# Deploy api\nuses helm").unwrap();
        std::fs::write(d.path().join("web.md"), "# Deploy web").unwrap();
        let mut v = NotesView::brain_at(d.path().to_path_buf());
        v.on_show();
        v.key(k(KeyCode::Char('/')));
        v.paste("helm");
        let s = screen(&mut v, 80, 10);
        assert!(s.contains("Deploy api") && !s.contains("Deploy web"), "{s}");
    }

    #[test]
    fn refresh_picks_up_new_files() {
        let d = tempfile::tempdir().unwrap();
        let mut v = NotesView::notepad_at(d.path().to_path_buf());
        v.on_show();
        std::fs::write(d.path().join("later.txt"), "Written later\n").unwrap();
        assert!(!screen(&mut v, 80, 8).contains("Written later"));
        v.refresh();
        assert!(screen(&mut v, 80, 8).contains("Written later"));
    }

    #[test]
    fn an_empty_brain_says_how_to_start() {
        let d = tempfile::tempdir().unwrap();
        let mut v = NotesView::brain_at(d.path().join("nothing-here"));
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
