//! `watch` — live supervision. Polls the registry, detects transitions, fires
//! macOS notifications on finished/stuck, and (on a TTY) renders a ratatui dashboard.
//!
//! The dashboard is used from a 200-column desktop terminal *and* from a phone
//! over mosh, so the interesting logic — key mapping, layout, row assembly — lives
//! in [`crate::tui::dash`] as pure functions with tests. What's left here is the loop.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListState, Paragraph, Wrap};
use serde::{Deserialize, Serialize};

use crate::cli::commands;
use crate::cli::render::{ago, width_of};
use crate::core::backend;
use crate::core::discovery::{Session, claude_home, discover, enrich_iterm_tabs, is_fixture};
use crate::core::naming::{GenOpts, NameCache, NameJob, NameMsg, NamePool, NameSource, Purpose};
use crate::error::{Error, Result};
use crate::tui::claw_card;
use crate::tui::dash::keys::{self, Action, RenameAction};
use crate::tui::dash::layout::{self as dashlayout, Density, Plan};
use crate::tui::dash::rows::{self, BearclawCtx, session_item};
use crate::tui::notify::notify;
use crate::tui::tabs::editor::{Editor, EditorOutcome};
use crate::tui::tabs::input::{InputResult, LineInput};
use crate::tui::tabs::session::{DetailOutcome, SessionDetail};
use crate::tui::tabs::{self, Outcome, Tab, Tabs};
use crate::tui::term::embed::Embedded;
use crate::tui::term::keys as term_keys;
use crate::tui::term::{clip, select};

const EVENT_CAP: usize = 20;

/// A session sitting idle on a permission/confirmation dialog is blocked on you.
fn awaiting(text: &str) -> bool {
    const MARKERS: [&str; 5] = [
        "Do you want to proceed",
        "Would you like to",
        "❯ 1. Yes",
        "❯ 1. Allow",
        "1. Yes,",
    ];
    MARKERS.iter().any(|m| text.contains(m))
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Prev {
    status: String,
    #[serde(default)]
    stuck_notified: bool,
}

type State = HashMap<String, Prev>;

fn state_path() -> PathBuf {
    claude_home().join("bearclaw-watch-state.json")
}
fn load_state() -> State {
    // Fixture mode is a demo: it must not inherit — or leave behind — transition
    // state for sessions that never existed.
    if is_fixture() {
        return State::default();
    }
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}
fn save_state(s: &State) {
    if is_fixture() {
        return;
    }
    if let Ok(t) = serde_json::to_string(s) {
        let _ = std::fs::write(state_path(), t);
    }
}

/// First 8 characters of a session key. Chars, not bytes: ids come from JSON
/// (the fixture's are user-supplied), and byte-slicing splits a codepoint.
fn short_key(key: &str) -> String {
    key.chars().take(8).collect()
}

struct Ev {
    icon: &'static str,
    time: String,
    msg: String,
}

impl Ev {
    fn now(icon: &'static str, msg: String) -> Self {
        Ev {
            icon,
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            msg,
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// One supervision pass: update `state`, fire notifications, return (rows, new events).
fn tick(state: &mut State, stuck_secs: i64) -> (Vec<Session>, Vec<Ev>) {
    let mut rows = crate::core::claw::without_claw(discover(), &crate::core::claw::dir());
    enrich_iterm_tabs(&mut rows);
    let mut evs = Vec::new();
    let time = chrono::Local::now().format("%H:%M:%S").to_string();
    let mut push = |icon: &'static str, msg: String| {
        evs.push(Ev {
            icon,
            time: time.clone(),
            msg,
        })
    };

    let mut seen = std::collections::HashSet::new();
    for r in &rows {
        let key = r.key();
        seen.insert(key.clone());
        let prev = state.get(&key).cloned().unwrap_or_default();
        let label = r.label();
        let known = state.contains_key(&key);

        if prev.status == "busy" && r.status == "idle" {
            let t = r.title.as_deref().unwrap_or("");
            push(
                "✅",
                format!(
                    "{label} finished — {}",
                    t.chars().take(40).collect::<String>()
                ),
            );
            notify(
                "Session finished",
                &format!("{label}: {}", t.chars().take(60).collect::<String>()),
            );
        } else if known && prev.status != "waiting" && r.is_waiting() {
            // The registry tells us this outright — no screen-scraping needed.
            let what = r.waiting_for.as_deref().unwrap_or("input needed");
            push("⏸", format!("{label} needs you — {what}"));
            notify("Session needs you", &format!("{label}: {what}"));
        } else if known && prev.status != "busy" && r.status == "busy" {
            push("▶", format!("{label} working"));
        } else if !known {
            push("＋", format!("{label} appeared"));
        }

        // Stuck: idle past the threshold and blocked on a prompt — check once per idle episode.
        let idle_secs = r.updated_at.map(|u| (now_ms() - u) / 1000).unwrap_or(0);
        let mut stuck_notified = prev.stuck_notified && r.status == "idle";
        // `is_fixture` short-circuits before `peek`: a canned bearclaw's handles are
        // fabricated, and shelling out to tmux/AppleScript with them is exactly
        // what a demo mode must not do.
        if r.status == "idle" && idle_secs > stuck_secs && !stuck_notified && !is_fixture() {
            let screen = backend::peek(r).unwrap_or_default();
            if awaiting(&screen) {
                push(
                    "⚠",
                    format!("{label} waiting for your input ({})", ago(r.updated_at)),
                );
                notify(
                    "Session waiting",
                    &format!("{label} is blocked on a prompt"),
                );
            }
            stuck_notified = true;
        }
        state.insert(
            key,
            Prev {
                status: r.status.clone(),
                stuck_notified: if r.status == "busy" {
                    false
                } else {
                    stuck_notified
                },
            },
        );
    }
    for key in state.keys().cloned().collect::<Vec<_>>() {
        if !seen.contains(&key) {
            push("✕", format!("{} closed", short_key(&key)));
            state.remove(&key);
        }
    }
    save_state(state);
    // A session blocked on the user is the one thing worth seeing first; sort_by_key
    // is stable, so within each group the registry's recency order survives.
    rows.sort_by_key(|r| !r.is_waiting());
    (rows, evs)
}

/// How `watch` was asked to run. `rows`/`mouse` are three-state: absent means
/// "whatever the config says".
#[derive(Default, Clone, Copy)]
pub struct WatchOpts {
    pub interval_secs: u64,
    pub stuck_secs: i64,
    pub quiet: bool,
    /// `None` = consult config; `Some(None)` = auto; `Some(Some(b))` = forced.
    pub rows: Option<Option<bool>>,
    /// `None` = consult config (which defaults to on).
    pub mouse: Option<bool>,
}

pub fn run(o: WatchOpts) -> Result<()> {
    let tui = std::io::IsTerminal::is_terminal(&io::stdout()) && !o.quiet;
    if tui {
        run_tui(o)
    } else {
        run_quiet(o.interval_secs, o.stuck_secs)
    }
}

fn run_quiet(interval_secs: u64, stuck_secs: i64) -> Result<()> {
    println!("bearclaw watch running (notify-only, every {interval_secs}s). Ctrl-c to stop.");
    let mut state = load_state();
    loop {
        let (_, evs) = tick(&mut state, stuck_secs);
        for e in evs {
            println!("{} {}  {}", e.icon, e.time, e.msg);
        }
        std::thread::sleep(Duration::from_secs(interval_secs));
    }
}

/// Restores the terminal on drop, so an early `?` return — or a panic — can't
/// leave it wedged in raw mode with the mouse still captured.
///
/// Teardown is the exact reverse of setup, and it shows the cursor again:
/// ratatui's `draw` hides it and nothing else ever brings it back, which leaves
/// the shell you return to with an invisible caret.
struct TermGuard {
    mouse: bool,
    /// Whether setup pushed the kitty keyboard-protocol flags, so teardown pops
    /// exactly what it pushed and never underflows a terminal that lacked them.
    enhanced: bool,
}
impl Drop for TermGuard {
    fn drop(&mut self) {
        if self.mouse {
            let _ = execute!(io::stdout(), DisableMouseCapture);
        }
        let _ = pop_keyboard_enhancement(&mut io::stdout(), self.enhanced);
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            Show
        );
        let _ = disable_raw_mode();
    }
}

/// Asks the outer terminal to disambiguate escape codes (the kitty keyboard
/// protocol's first level) when it advertises support, so modified keys —
/// Shift+Enter above all, but also Ctrl+… and Alt+… — arrive as distinct events
/// instead of collapsing onto their legacy bytes. Without it iTerm delivers
/// Shift+Enter as a bare `KeyCode::Enter` with no `SHIFT`, and the embedded
/// program never gets the ESC-CR that Claude Code reads as "newline, don't send".
///
/// Only `DISAMBIGUATE_ESCAPE_CODES` is requested — not event-type reporting — so
/// no key-release or repeat events start arriving. Returns whether the flags were
/// pushed, which the teardown guard uses to pop precisely what was pushed.
fn push_keyboard_enhancement<W: io::Write>(w: &mut W, supported: bool) -> io::Result<bool> {
    if supported {
        execute!(
            w,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    Ok(supported)
}

/// Undoes [`push_keyboard_enhancement`]. A no-op when nothing was pushed, so a
/// terminal that never received the flags doesn't have its protocol stack popped.
fn pop_keyboard_enhancement<W: io::Write>(w: &mut W, enhanced: bool) -> io::Result<()> {
    if enhanced {
        execute!(w, PopKeyboardEnhancementFlags)?;
    }
    Ok(())
}

/// `"1"`/`"2"`/`"auto"` from the config, folded with the command-line override.
///
/// `2` and `auto` both mean the full item — two lines on a desktop, three on a
/// phone — so `1`, the compact single line `z` toggles into, is the only value
/// that changes anything today.
fn resolve_rows(flag: Option<Option<bool>>) -> Option<bool> {
    if let Some(explicit) = flag {
        return explicit;
    }
    match commands::ui_config().rows.as_deref() {
        Some("1") => Some(false),
        Some("2") => Some(true),
        _ => None,
    }
}

fn rows_setting(force_two_row: Option<bool>) -> &'static str {
    match force_two_row {
        Some(true) => "2",
        Some(false) => "1",
        None => "auto",
    }
}

/// Everything the dashboard keeps between frames.
struct Dash {
    list: ListState,
    help: bool,
    /// `None` = size-driven, `Some(b)` = the `z` toggle / `--rows` flag.
    force_two_row: Option<bool>,
    /// `None` = size-driven, `Some(b)` = the `e` toggle.
    events: Option<bool>,
    /// Recorded on every draw so a click can be mapped back to a row. Zeroed
    /// while the help overlay is up, so a tap on the overlay can't select and
    /// focus the row underneath it.
    list_area: Rect,
    item_height: u16,
    /// Whether the last frame actually drew the events pane — `e` toggles
    /// relative to what's on screen, not to the stored override.
    events_shown: bool,
    /// Whether the last frame was a Tiny pane, where the row-mode override is
    /// ignored — so `z` doesn't persist a setting the user never saw applied.
    tiny: bool,
    /// `Some` while naming jobs are in flight — drives the header spinner.
    naming: Option<NameProgress>,
    /// Frame counter, only ever used to advance the spinner.
    tick: u64,
    /// Sessions a reboot left dormant on this machine (`bearclaw restore`), re-read every poll.
    dormant: usize,
    /// Each tab's columns on the bar (row 0), recorded on every draw for clicks.
    tab_hits: Vec<(Tab, u16, u16)>,
    /// The refresh button's `[x0, x1)` on the bar, when it fit.
    refresh_hit: Option<(u16, u16)>,
    /// `Some` while one session is open up close (`d`), in place of the list.
    detail: Option<SessionDetail>,
    /// `Some` while a file is open in the editor, over whichever tab asked for it.
    editor: Option<Editor>,
    /// `Some` while a tmux session is open inside the dashboard (Sessions tab).
    term: Option<Term>,
    /// Where the open terminal was last drawn — mouse wheel coordinates are relative to it.
    term_area: Rect,
    /// `c`: the directory prompt for a new session.
    new_session: Option<LineInput>,
    /// The first prompt for that session, when a todo item started it.
    new_prompt: Option<String>,
    /// Sessions' todo lines and sticky notes, by session id.
    notes: HashMap<String, SessionNote>,
    /// Claw's terminal, once its session is up (Claw tab).
    claw: Option<Term>,
    /// Claw's session being found or started in the background.
    claw_starting: Option<Receiver<std::result::Result<String, String>>>,
    /// Why Claw could not start (`r` on its tab retries).
    claw_error: Option<String>,
    /// The Claw character card's stats, rebuilt on a timer so the file reads behind it
    /// (todo, brain) don't run every frame.
    claw_card: Option<(Instant, claw_card::Stats)>,
    /// A new session being started in the background: its tmux session name, or why not.
    spawning: Option<Receiver<std::result::Result<String, String>>>,
}

/// A tmux session drawn inside the dashboard.
struct Term {
    session: String,
    /// Its Claude session id, when it was opened from the list — for its todo line.
    id: Option<String>,
    embed: Embedded,
    /// The mouse text selection over its screen, if any.
    sel: select::Selection,
}

/// What a session keeps about itself for the dashboard: its todo line and sticky note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionNote {
    todo: Option<String>,
    bullets: Vec<String>,
}

/// Every listed session's note, read once per poll (two small files, not per frame).
fn load_notes(rows: &[Session]) -> HashMap<String, SessionNote> {
    let todo = crate::core::todo::load(&crate::core::todo::path()).unwrap_or_default();
    let dir = crate::core::sticky::dir();
    rows.iter()
        .filter_map(|r| {
            let id = r.session_id.as_ref()?;
            let note = SessionNote {
                todo: todo.doing_for(id).map(|i| i.text.clone()),
                bullets: crate::core::sticky::get(&dir, id)
                    .map(|s| s.bullets)
                    .unwrap_or_default(),
            };
            (note != SessionNote::default()).then(|| (id.clone(), note))
        })
        .collect()
}

/// An open rename buffer, pinned to the session it was started on.
struct Rename {
    /// The session's stable key, *not* its row index. Rows are re-polled and
    /// re-sorted while the buffer is open, so an index would happily send
    /// `/rename` into whatever had drifted into that slot.
    key: String,
    buf: String,
}

/// Resolve a pinned rename against the current rows and send it. `None` = the
/// buffer was empty, so there is nothing to say about it.
///
/// `sync_tmux` carries the `[naming] sync_tmux` setting through: a confirmed
/// rename is also where the session's tmux session name gets brought in line.
/// The third field says whether `/rename` actually went out: a session that was
/// waiting on a prompt is *held*, and its pending suggestion has to survive for the next
/// attempt rather than being consumed by one that never happened.
fn commit_rename(
    r: &Rename,
    rows: &[Session],
    sync_tmux: bool,
) -> Option<(&'static str, String, bool)> {
    if r.buf.trim().is_empty() {
        return None;
    }
    // The CLI verb validates and crops; the typed buffer has to as well, or a
    // 300-character line goes out as `/rename <300 chars>` and lands as a
    // 300-character tmux session name.
    let name = match commands::clean_name(&r.buf) {
        Ok(n) => n,
        Err(e) => return Some(("✕", format!("rename dropped: {e}"), false)),
    };
    let name = name.as_str();
    let Some(s) = rows.iter().find(|s| s.key() == r.key) else {
        return Some((
            "✕",
            format!("rename dropped — session {} is gone", short_key(&r.key)),
            true,
        ));
    };
    let opts = commands::RenameOpts {
        sync_tmux,
        // The dashboard has no modifier for it; `tb-bearclaw rename --force` is the
        // deliberate path for a session that is always waiting.
        force: false,
    };
    Some(match commands::apply_rename(s, name, opts) {
        Ok(outcome @ commands::RenameOutcome::Sent(_)) => match outcome.tmux_note() {
            None => ("✎", format!("{} → {name}", s.headline()), true),
            Some(note) => ("✎", format!("{} → {name}  ({note})", s.headline()), true),
        },
        Ok(commands::RenameOutcome::Held(_, why)) => ("⏸", why, false),
        Err(e) => ("✕", format!("{} → {name} failed: {e}", s.label()), false),
    })
}

// --- naming ------------------------------------------------------------------

/// Header progress for an in-flight naming pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NameProgress {
    done: usize,
    total: usize,
}

/// Everything the naming feature keeps between frames.
///
/// Two things come out of the same generator here. **Titles** are what every row
/// draws on its first line, and they are automatic: the dashboard titles the
/// whole bearclaw in the background so a row says what its session is doing without
/// anyone pressing anything. **Suggestions** are the `N` flow — the same title,
/// offered as a new Claude session *name*, which only a confirmed `/rename`
/// applies. A title is display state and costs nothing to be wrong about; a name
/// is typed into a live session, so it keeps its confirmation step.
struct Naming {
    pool: Option<(NamePool, Receiver<NameMsg>)>,
    /// Generated titles by session key — the headline source for every row.
    titles: HashMap<String, String>,
    /// What each title was generated from (the session's first prompt), so a
    /// title made before the user had said anything is regenerated once they do.
    /// The transcript's first prompt is the one input that lands *after* a
    /// session shows up in the registry.
    titled_from: HashMap<String, Option<String>>,
    /// Names generated this run, each still awaiting the user's ⏎.
    suggestions: HashMap<String, String>,
    /// In flight, so a second `N` on the same row doesn't queue it twice.
    queued: HashSet<String>,
    /// Keys the user asked to be *offered* while a background title for them was
    /// already in flight. The job in the channel can't be amended, so the answer
    /// is upgraded on arrival instead — without this, `N` pressed during the
    /// startup titling pass looked like it did nothing at all.
    wanted: HashMap<String, Purpose>,
    /// Already fell back to the heuristic once — the model is having a bad day
    /// and re-queueing it every keypress just burns time.
    exhausted: HashSet<String>,
    pending: usize,
    total: usize,
    enabled: bool,
    /// Title the bearclaw without being asked. `[naming] auto_title = false` turns
    /// it off, and rows fall back to the Claude session name.
    auto: bool,
    model: String,
    /// The name cache as it was on disk at startup, read once. Seeds titles for
    /// sessions the workers haven't answered for yet.
    disk: NameCache,
}

impl Naming {
    fn new(cfg: &commands::NamingConfig) -> Self {
        // Last run's titles, so the dashboard opens *already* reading as itself
        // instead of showing a screen of `app-9d` until the first model call
        // lands. Every one of them is re-checked by the titling pass below.
        let disk = NameCache::load();
        Naming {
            pool: None,
            titles: HashMap::new(),
            titled_from: HashMap::new(),
            suggestions: HashMap::new(),
            queued: HashSet::new(),
            wanted: HashMap::new(),
            exhausted: HashSet::new(),
            pending: 0,
            total: 0,
            // A canned bearclaw must never reach the model: its sessions are
            // fabricated, so every call would be paid for and meaningless.
            enabled: cfg.enabled() && !is_fixture(),
            auto: cfg.auto_title(),
            model: cfg.model(),
            disk,
        }
    }

    /// Title everything the bearclaw is showing, quietly. Runs after every poll and
    /// is a no-op once the bearclaw is titled.
    fn autotitle(&mut self, rows: &[Session]) {
        if !self.auto || !self.enabled {
            return;
        }
        self.adopt_disk_titles(rows);
        for key in self.due_for_title(rows) {
            let Some(s) = rows.iter().find(|s| s.key() == key).cloned() else {
                continue;
            };
            if self.spawn(&s, Purpose::Title) {
                self.titled_from.insert(key, s.title.clone());
            }
        }
    }

    /// Take last run's titles for rows that have none yet, so the dashboard reads
    /// as itself from the first frame instead of showing `app-9d` until a model
    /// call lands. Adopted titles are still queued below — the worker's cache
    /// read confirms them for free when the input hasn't moved.
    fn adopt_disk_titles(&mut self, rows: &[Session]) {
        for s in rows {
            let key = s.key();
            if self.titles.contains_key(&key) {
                continue;
            }
            if let Some(name) = self.disk.title(&key) {
                self.titles.insert(key, name.to_string());
            }
        }
    }

    /// Which rows are due a title.
    ///
    /// A session is due when this run hasn't titled it, or when its first prompt
    /// has changed since the title it has was generated from — the transcript's
    /// first prompt is the one naming input that lands *after* the session shows
    /// up in the registry, so a session titled at boot from a directory alone
    /// gets a real title the moment the user says what they want.
    ///
    /// Pure, so the policy can be exercised without starting a pool or paying
    /// for a model call.
    fn due_for_title(&self, rows: &[Session]) -> Vec<String> {
        rows.iter()
            .map(|s| (s.key(), s))
            .filter(|(key, s)| {
                !self.queued.contains(key)
                    && !self.exhausted.contains(key)
                    // Titled this run already; only a changed prompt re-opens it.
                    && self.titled_from.get(key).is_none_or(|from| from != &s.title)
            })
            .map(|(key, _)| key)
            .collect()
    }

    /// Queue one session, starting the pool if this is the first job.
    ///
    /// The pool is started on demand rather than at boot because it isn't always
    /// wanted: `[naming] enabled = false` and fixture mode never get here, and
    /// two idle threads plus a channel are not free.
    fn spawn(&mut self, s: &Session, purpose: Purpose) -> bool {
        if self.pool.is_none() {
            // `enabled` is the caller's to check; the TUI has no refresh key,
            // `tb-bearclaw name --refresh` does.
            self.pool = Some(NamePool::start(self.model.clone(), GenOpts::llm()));
        }
        let Some((pool, _)) = &self.pool else {
            return false;
        };
        let key = s.key();
        if !pool.enqueue(NameJob {
            key: key.clone(),
            session: s.clone(),
            purpose,
        }) {
            return false;
        }
        self.queued.insert(key);
        self.pending += 1;
        self.total += 1;
        true
    }

    fn progress(&self) -> Option<NameProgress> {
        (self.pending > 0).then_some(NameProgress {
            done: self.total.saturating_sub(self.pending),
            total: self.total,
        })
    }

    /// Queue `keys` for naming on the user's behalf (`N` / `Ctrl-N`). Returns the
    /// events to log, plus the keys whose name is already in hand and can be
    /// offered straight away.
    ///
    /// A title generated by the background pass counts as in hand: by the time
    /// anyone presses `N` the bearclaw is usually titled already, so the ordinary
    /// `N` costs nothing and never queues behind a titling backlog.
    fn request(
        &mut self,
        rows: &[Session],
        keys: &[String],
        purpose: Purpose,
    ) -> (Vec<Ev>, Vec<String>) {
        let mut evs = Vec::new();
        let mut ready = Vec::new();
        if keys.is_empty() {
            evs.push(Ev::now("·", "nothing to name".into()));
            return (evs, ready);
        }
        if !self.enabled {
            evs.push(Ev::now(
                "·",
                "naming is off here (fixture mode, or [naming] enabled = false)".into(),
            ));
            return (evs, ready);
        }
        let mut queued_now = 0usize;
        for key in keys {
            if self.titles.contains_key(key) {
                self.suggestions
                    .entry(key.clone())
                    .or_insert_with(|| self.titles[key].clone());
                ready.push(key.clone());
                continue;
            }
            if self.queued.contains(key) {
                self.wanted.insert(key.clone(), purpose);
                continue;
            }
            let Some(s) = rows.iter().find(|s| &s.key() == key) else {
                continue;
            };
            if self.exhausted.contains(key) {
                evs.push(Ev::now(
                    "·",
                    format!("{} already fell back once this run", s.label()),
                ));
                continue;
            }
            let s = s.clone();
            if self.spawn(&s, purpose) {
                queued_now += 1;
            }
        }
        if queued_now > 0 {
            evs.push(Ev::now(
                "…",
                format!(
                    "naming {queued_now} session{}",
                    if queued_now == 1 { "" } else { "s" }
                ),
            ));
        }
        (evs, ready)
    }

    /// Everything the workers have answered since the last frame. Never blocks —
    /// this runs at the top of the draw loop.
    fn drain(&mut self) -> Vec<NameMsg> {
        let mut out = Vec::new();
        if let Some((_, rx)) = &self.pool {
            while let Ok(msg) = rx.try_recv() {
                out.push(msg);
            }
        }
        out
    }

    /// What the answer to `key` should do now it's here: whatever the job was
    /// queued as, unless the user asked for more while it was in flight.
    fn purpose_of(&mut self, key: &str, queued_as: Purpose) -> Purpose {
        match self.wanted.remove(key) {
            Some(upgraded) if !queued_as.is_interactive() => upgraded,
            _ => queued_as,
        }
    }

    /// Book-keeping for one answered job.
    fn settle(&mut self, key: &str, fell_back: bool) {
        self.queued.remove(key);
        self.pending = self.pending.saturating_sub(1);
        if self.pending == 0 {
            self.total = 0;
        }
        if fell_back {
            self.exhausted.insert(key.to_string());
        }
    }
}

/// Bring a suggestion to the user.
///
/// Prefills the pinned rename buffer when the session can actually take a
/// `/rename` — and holds it, with a reason, when it can't. A session sitting on
/// a permission prompt would read the rename as its answer (see
/// [`crate::core::title::Hold`]), and a vanished session is dropped rather than
/// retargeted at whoever took its row.
fn offer(key: &str, name: &str, rows: &[Session], renaming: &mut Option<Rename>, bulk: bool) -> Ev {
    let Some(s) = rows.iter().find(|s| s.key() == key) else {
        return Ev::now(
            "✕",
            format!("suggestion dropped — session {} is gone", short_key(key)),
        );
    };
    let label = s.label();
    if crate::core::title::hold(s).is_some() {
        return Ev::now(
            "⏸",
            format!("{label} → {name} held ({}) — N when it's idle", s.status),
        );
    }
    // A bulk pass must not open twelve buffers in a row, and a buffer already
    // open belongs to whatever the user is typing right now.
    if bulk || renaming.is_some() {
        return Ev::now("✎", format!("{label} → {name} — press N to apply"));
    }
    *renaming = Some(Rename {
        key: key.to_string(),
        buf: name.to_string(),
    });
    Ev::now("✎", format!("{label} → {name}? ⏎ applies"))
}

fn spinner_frame(tick: u64) -> &'static str {
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[(tick as usize) % FRAMES.len()]
}

/// What one terminal event means to an open rename buffer.
///
/// Everything that isn't a key press maps to [`RenameAction::None`] — mouse
/// events in particular. They used to fall through to `mouse_action`, which
/// moved the selection *and* focused another terminal, and then the pending
/// `/rename` was typed into that session.
fn rename_event(ev: &Event) -> RenameAction {
    match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => keys::rename_action(*k),
        _ => RenameAction::None,
    }
}

/// A `last_poll` stamp that makes the next poll land `after` from now.
/// `Instant::now() - interval` panics on a machine booted less than `interval`
/// ago, which a generous `--interval` makes reachable.
fn poll_again_in(interval: Duration, after: Duration) -> Instant {
    let back = interval.saturating_sub(after);
    Instant::now()
        .checked_sub(back)
        .unwrap_or_else(Instant::now)
}

fn run_tui(o: WatchOpts) -> Result<()> {
    // One read of config.toml for the whole startup: `[ui]`, `[naming]` and the
    // parse error all come out of it.
    let loaded = commands::load_all();
    let mouse = o
        .mouse
        .or(loaded.cfg.ui.mouse)
        // On by default: a tap is the only pointing device on a phone.
        .unwrap_or(true);

    enable_raw_mode().map_err(Error::Io)?;
    // Probe before the alternate screen and the event loop: the terminal answers
    // the query as input, and we want that reply consumed here, not mid-dashboard.
    let supports_enhancement = supports_keyboard_enhancement().unwrap_or(false);
    execute!(io::stdout(), EnterAlternateScreen).map_err(Error::Io)?;
    let enhanced =
        push_keyboard_enhancement(&mut io::stdout(), supports_enhancement).map_err(Error::Io)?;
    let _guard = TermGuard { mouse, enhanced };
    if mouse {
        execute!(io::stdout(), EnableMouseCapture).map_err(Error::Io)?;
    }
    // A paste arrives whole, so a multi-line one can't press Enter in a session line by line.
    execute!(io::stdout(), EnableBracketedPaste).map_err(Error::Io)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout())).map_err(Error::Io)?;

    let mut state = load_state();
    let mut log: Vec<Ev> = Vec::new();
    let mut rows: Vec<Session> = Vec::new();
    let mut last_poll: Option<Instant> = None;
    // Track selection by session key, not index, so it stays put as rows re-sort.
    let mut selected: Option<String> = None;
    // Some(..) while typing a new name for a pinned session.
    let mut renaming: Option<Rename> = None;
    let interval = Duration::from_secs(o.interval_secs);
    let mut dash = Dash {
        list: ListState::default(),
        help: false,
        force_two_row: resolve_rows(o.rows),
        events: None,
        list_area: Rect::default(),
        item_height: 1,
        events_shown: false,
        tiny: false,
        naming: None,
        tick: 0,
        dormant: 0,
        tab_hits: Vec::new(),
        refresh_hit: None,
        detail: None,
        editor: None,
        term: None,
        term_area: Rect::default(),
        new_session: None,
        new_prompt: None,
        notes: HashMap::new(),
        claw: None,
        claw_starting: None,
        claw_error: None,
        claw_card: None,
        spawning: None,
    };
    let mut tabs = Tabs::new();
    let mut naming = Naming::new(&loaded.cfg.naming);
    let sync_tmux = loaded.cfg.naming.sync_tmux();
    // A config that doesn't parse degrades to defaults in silence — say so once,
    // otherwise `z` appearing not to stick has no visible explanation.
    if let Some(problem) = loaded.problem {
        log.push(Ev::now("⚠", problem));
    }

    loop {
        // The naming workers answer on their own schedule; take whatever they
        // have before drawing, so a result never waits a frame to show up.
        for msg in naming.drain() {
            let ev = match msg {
                // Said once per pool by contract — `claude` missing, logged out
                // or rate-limited is one line, not one line per session.
                NameMsg::Unavailable(why) => Some(Ev::now("⚠", format!("naming fell back: {why}"))),
                NameMsg::Failed {
                    key,
                    label,
                    err,
                    purpose,
                } => {
                    let purpose = naming.purpose_of(&key, purpose);
                    naming.settle(&key, true);
                    // A background title that didn't work out is not news: the
                    // row keeps its session name and `Unavailable` has already
                    // said whatever systemic thing went wrong, once.
                    purpose
                        .is_interactive()
                        .then(|| Ev::now("✕", format!("no name for {label}: {err}")))
                }
                NameMsg::Named {
                    key,
                    name,
                    source,
                    purpose,
                    ..
                } => {
                    let purpose = naming.purpose_of(&key, purpose);
                    naming.settle(&key, source == NameSource::Heuristic);
                    // Every answer becomes the row's headline, however it was
                    // asked for — one generation serves the display and the
                    // rename flow both.
                    naming.titles.insert(key.clone(), name.clone());
                    purpose.is_interactive().then(|| {
                        naming.suggestions.insert(key.clone(), name.clone());
                        offer(
                            &key,
                            &name,
                            &rows,
                            &mut renaming,
                            // Over an open terminal a rename buffer would be invisible and
                            // steal its keys: say it in the log instead.
                            purpose == Purpose::Bulk
                                || dash.term.is_some()
                                || tabs.active != Tab::Sessions,
                        )
                    })
                }
            };
            if let Some(ev) = ev {
                log.insert(0, ev);
                log.truncate(EVENT_CAP);
            }
        }
        dash.naming = naming.progress();

        if last_poll.map(|t| t.elapsed() >= interval).unwrap_or(true) {
            let (r, mut evs) = tick(&mut state, o.stuck_secs);
            rows = r;
            evs.reverse(); // newest first when prepended
            for e in evs {
                log.insert(0, e);
            }
            log.truncate(EVENT_CAP);
            last_poll = Some(Instant::now());
            dash.dormant = crate::core::snapshot::dormant_count();
            dash.notes = load_notes(&rows);
            tabs.live(&rows);
            // New sessions get a title, and a session whose first prompt has
            // landed since gets a better one. A no-op once the bearclaw is titled.
            naming.autotitle(&rows);
        }
        // The registry knows nothing about titles, and `rows` came from it.
        apply_titles(&mut rows, &naming.titles);
        if selected.is_none() && !rows.is_empty() {
            selected = Some(rows[0].key());
        }
        // The selected session can vanish between polls; fall back to the top.
        let sel = selected
            .as_ref()
            .and_then(|k| rows.iter().position(|r| &r.key() == k))
            .unwrap_or(0);
        // ListState::offset survives a shrinking list and can point past the end.
        clamp_offset(&mut dash, rows.len());

        tabs.tick_all();
        if let Some(t) = dash.term.as_mut() {
            t.embed.pump();
        }
        if tabs.active == Tab::Claw
            && dash.claw.is_none()
            && dash.claw_starting.is_none()
            && dash.claw_error.is_none()
        {
            dash.claw_starting = Some(start_claw());
        }
        if let Some(rx) = &dash.claw_starting
            && let Ok(result) = rx.try_recv()
        {
            dash.claw_starting = None;
            let (cols, height) = crossterm::terminal::size().unwrap_or((80, 24));
            match result.and_then(|name| {
                Embedded::attach(&name, height.saturating_sub(2), cols).map(|mut embed| {
                    // Claw shares the pane with the character card; pin its window so it always
                    // gets exactly the width we draw into, even if a second client is attached.
                    embed.pin();
                    Term {
                        session: name,
                        id: None,
                        embed,
                        sel: select::Selection::default(),
                    }
                })
            }) {
                Ok(t) => dash.claw = Some(t),
                Err(e) => dash.claw_error = Some(e),
            }
        }
        if let Some(t) = dash.claw.as_mut() {
            t.embed.pump();
        }
        if let Some(rx) = &dash.spawning
            && let Ok(result) = rx.try_recv()
        {
            dash.spawning = None;
            match result {
                Ok(session) => {
                    open_tmux(&session, None, &mut dash, &mut log);
                    // The new session should show up in the list right away.
                    last_poll = None;
                }
                Err(e) => {
                    log.insert(0, Ev::now("✕", format!("new session: {e}")));
                    log.truncate(EVENT_CAP);
                }
            }
        }
        if let Some(d) = dash.detail.as_mut()
            && let Some(msg) = d.tick(&rows)
        {
            log.insert(0, Ev::now("→", msg));
            log.truncate(EVENT_CAP);
        }
        terminal
            .draw(|f| {
                draw_app(
                    f,
                    &mut tabs,
                    &rows,
                    &log,
                    sel,
                    renaming.as_ref(),
                    &o,
                    &mut dash,
                )
            })
            .map_err(Error::Io)?;

        // A live terminal redraws at screen pace; the dashboard alone is calmer.
        let wait = if dash.term.is_some()
            || dash.spawning.is_some()
            || dash.claw_starting.is_some()
            || (tabs.active == Tab::Claw && dash.claw.is_some())
        {
            16
        } else {
            250
        };
        if !event::poll(Duration::from_millis(wait)).map_err(Error::Io)? {
            continue;
        }
        let ev = event::read().map_err(Error::Io)?;

        // Renaming swallows the whole input stream until it's applied or
        // cancelled — mouse events included. A stray tap used to fall through to
        // `mouse_action`, move the selection *and* focus another terminal, and
        // the pending `/rename` then landed in that session.
        if renaming.is_some() {
            if let Event::Resize(_, _) = ev {
                clamp_offset(&mut dash, rows.len());
            }
            if let Event::Paste(text) = &ev
                && let Some(r) = renaming.as_mut()
            {
                r.buf.push_str(&text.lines().collect::<Vec<_>>().join(" "));
                continue;
            }
            match rename_event(&ev) {
                RenameAction::None => {}
                RenameAction::Insert(c) => {
                    if let Some(r) = renaming.as_mut() {
                        r.buf.push(c);
                    }
                }
                RenameAction::Backspace => {
                    if let Some(r) = renaming.as_mut() {
                        r.buf.pop();
                    }
                }
                RenameAction::Cancel => renaming = None,
                RenameAction::Quit => break,
                RenameAction::Commit => {
                    if let Some(r) = renaming.take()
                        && let Some((icon, msg, sent)) = commit_rename(&r, &rows, sync_tmux)
                    {
                        if sent {
                            naming.suggestions.remove(&r.key);
                        }
                        log.insert(0, Ev::now(icon, msg));
                        log.truncate(EVENT_CAP);
                        // Claude writes the new name on its next status update.
                        last_poll = Some(poll_again_in(interval, Duration::from_secs(2)));
                    }
                }
            }
            continue;
        }

        // An open editor owns every key until it closes.
        if let Some(ed) = dash.editor.as_mut() {
            if let Event::Paste(text) = &ev {
                ed.paste(text);
            } else if let Event::Key(k) = ev
                && k.kind != KeyEventKind::Release
                && ed.key(k) == EditorOutcome::Closed
            {
                dash.editor = None;
                // Whatever was edited is re-read when its tab shows again.
                let active = tabs.active;
                tabs.switch(active);
            }
            continue;
        }

        // The open terminal owns the Sessions tab: every key goes into it except Ctrl+L.
        if tabs.active == Tab::Sessions
            && let Some(t) = dash.term.as_mut()
        {
            match term_input(t, &ev, dash.term_area, &dash.tab_hits) {
                TermInput::Leave => dash.term = None,
                TermInput::Switch(to) => tabs.switch(to),
                TermInput::Step(next) => tabs.switch(step(tabs.active, next)),
                TermInput::Copy(text) => copy_selection(&text, &mut log),
                TermInput::None => {}
            }
            continue;
        }
        // Claw's terminal is its tab; leaving goes back to Sessions, a dead one restarts.
        if tabs.active == Tab::Claw
            && let Some(t) = dash.claw.as_mut()
        {
            let dead = !t.embed.alive();
            match term_input(t, &ev, dash.term_area, &dash.tab_hits) {
                TermInput::Leave if dead => dash.claw = None,
                TermInput::Leave => tabs.switch(Tab::Sessions),
                TermInput::Switch(to) => tabs.switch(to),
                TermInput::Step(next) => tabs.switch(step(tabs.active, next)),
                TermInput::Copy(text) => copy_selection(&text, &mut log),
                TermInput::None => {}
            }
            continue;
        }
        if let Some(input) = dash.new_session.as_mut() {
            if let Event::Paste(text) = &ev {
                input.paste(text);
            } else if let Event::Key(k) = ev
                && k.kind != KeyEventKind::Release
            {
                match input.key(k) {
                    InputResult::Editing => {}
                    InputResult::Cancel => {
                        dash.new_session = None;
                        dash.new_prompt = None;
                    }
                    InputResult::Quit => break,
                    InputResult::Submit(dir) => {
                        dash.new_session = None;
                        let dir = crate::core::tools::expand_tilde(dir.trim());
                        log.insert(0, Ev::now("→", format!("starting a session in {dir}…")));
                        log.truncate(EVENT_CAP);
                        dash.spawning = Some(spawn_in_tmux(dir, dash.new_prompt.take()));
                    }
                }
            }
            continue;
        }

        // The bar's refresh button: the active tab fetches now; Sessions re-polls.
        if let Event::Mouse(m) = &ev
            && m.kind == MouseEventKind::Down(MouseButton::Left)
            && m.row == 0
            && dash
                .refresh_hit
                .is_some_and(|(x0, x1)| m.column >= x0 && m.column < x1)
        {
            let active = tabs.active;
            match tabs.view(active) {
                Some(v) => v.refresh(),
                None => last_poll = None,
            }
            continue;
        }

        // Tab / ⇧Tab and a tap on the bar switch tabs, unless a text input owns the keys.
        let detail_typing = dash.detail.as_ref().is_some_and(SessionDetail::typing);
        if !tabs.typing() && !detail_typing {
            let to = match &ev {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    tabs::tab_step(*k).map(|next| {
                        if next {
                            tabs.active.next()
                        } else {
                            tabs.active.prev()
                        }
                    })
                }
                Event::Mouse(m)
                    if m.kind == MouseEventKind::Down(MouseButton::Left) && m.row == 0 =>
                {
                    tabs::hit(&dash.tab_hits, m.column)
                }
                _ => None,
            };
            if let Some(t) = to {
                tabs.switch(t);
                continue;
            }
        }
        // Claw is still starting (or failed to): only its own few keys mean anything.
        if tabs.active == Tab::Claw {
            if let Event::Key(k) = ev
                && k.kind != KeyEventKind::Release
            {
                match k.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => tabs.switch(Tab::Sessions),
                    KeyCode::Char('r') => dash.claw_error = None,
                    _ => {}
                }
            }
            continue;
        }
        let active = tabs.active;
        if let Some(view) = tabs.view(active) {
            let outcome = match ev {
                Event::Key(k) if k.kind != KeyEventKind::Release => view.key(k),
                Event::Paste(text) => {
                    view.paste(&text);
                    Outcome::None
                }
                Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                    view.click(m.column, m.row)
                }
                _ => Outcome::None,
            };
            match outcome {
                Outcome::None => {}
                Outcome::Quit => break,
                Outcome::Home => tabs.switch(Tab::Sessions),
                Outcome::Log(msg) => {
                    log.insert(0, Ev::now("·", msg));
                    log.truncate(EVENT_CAP);
                }
                Outcome::Session(id) => {
                    tabs.switch(Tab::Sessions);
                    match rows
                        .iter()
                        .find(|r| r.session_id.as_deref() == Some(id.as_str()))
                    {
                        Some(r) => {
                            selected = Some(r.key());
                            let r = r.clone();
                            open_session(&r, &mut dash, &mut log);
                        }
                        None => {
                            log.insert(0, Ev::now("·", "that session has ended".into()));
                            log.truncate(EVENT_CAP);
                        }
                    }
                }
                // Ask where first: the directory prompt is also the "yes, start it".
                Outcome::Spawn(prompt) => {
                    tabs.switch(Tab::Sessions);
                    let mut input = LineInput::new("start a session on it in");
                    input.buf = default_session_dir();
                    dash.new_session = Some(input);
                    dash.new_prompt = Some(prompt);
                }
                Outcome::Edit(path) => match Editor::open(path) {
                    Ok(e) => dash.editor = Some(e),
                    Err(e) => {
                        log.insert(0, Ev::now("✕", format!("editor: {e}")));
                        log.truncate(EVENT_CAP);
                    }
                },
            }
            continue;
        }

        // A session open up close owns the Sessions tab's keys until it closes.
        if let Some(d) = dash.detail.as_mut() {
            if let Event::Paste(text) = &ev {
                d.paste(text);
            } else if let Event::Key(k) = ev
                && k.kind != KeyEventKind::Release
            {
                match d.key(k, &rows) {
                    DetailOutcome::None => {}
                    DetailOutcome::Close => dash.detail = None,
                    DetailOutcome::Quit => break,
                    DetailOutcome::Log(msg) => {
                        log.insert(0, Ev::now("·", msg));
                        log.truncate(EVENT_CAP);
                    }
                }
            }
            continue;
        }

        let action = match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => keys::action(k),
            Event::Mouse(m) => {
                keys::mouse_action(m, dash.list_area, dash.list.offset(), dash.item_height)
            }
            // A resize needs a redraw (the loop does that anyway) and a re-clamp.
            Event::Resize(_, _) => {
                clamp_offset(&mut dash, rows.len());
                Action::None
            }
            _ => Action::None,
        };

        match action {
            Action::None => {}
            // Esc doubles as "close the overlay" before it means "quit".
            Action::Quit if dash.help => dash.help = false,
            Action::Quit => break,
            Action::Refresh => last_poll = None,
            // Pin the target now: `sel` is recomputed from the live rows every
            // iteration, so by the time ⏎ lands it may point at someone else.
            Action::Rename => {
                if let Some(r) = rows.get(sel) {
                    renaming = Some(Rename {
                        key: r.key(),
                        buf: String::new(),
                    });
                }
            }
            Action::ToggleHelp => dash.help = !dash.help,
            Action::NewSession => {
                let mut input = LineInput::new("new session in");
                input.buf = default_session_dir();
                dash.new_session = Some(input);
            }
            Action::Detail => {
                if let Some(r) = rows.get(sel) {
                    dash.detail = Some(SessionDetail::open(r.key()));
                }
            }
            Action::ToggleEvents => dash.events = Some(!dash.events_shown),
            // A Tiny pane has no second line to give, so `plan` ignores the
            // override there — persisting it would strand the user in a mode they
            // never chose the next time they open a wide terminal.
            Action::ToggleRows if dash.tiny => {
                log.insert(0, Ev::now("·", "the row layout needs a wider pane".into()));
                log.truncate(EVENT_CAP);
            }
            Action::ToggleRows => {
                // Flip relative to what's on screen, then remember it.
                dash.force_two_row = Some(dash.item_height != 2);
                if let Err(e) = commands::persist_ui("rows", rows_setting(dash.force_two_row)) {
                    log.insert(0, Ev::now("✕", format!("layout not remembered: {e}")));
                    log.truncate(EVENT_CAP);
                }
            }
            Action::Up if !rows.is_empty() => {
                selected = Some(rows[sel.saturating_sub(1)].key());
            }
            Action::Down if !rows.is_empty() => {
                selected = Some(rows[(sel + 1).min(rows.len() - 1)].key());
            }
            Action::Up | Action::Down => {}
            Action::Focus => {
                if let Some(r) = rows.get(sel).cloned() {
                    open_session(&r, &mut dash, &mut log);
                }
            }
            // Digit and tap indices are raw — re-clamp before trusting them.
            Action::JumpTo(idx) if idx < rows.len() => {
                selected = Some(rows[idx].key());
                let r = rows[idx].clone();
                open_session(&r, &mut dash, &mut log);
            }
            Action::JumpTo(_) => {}
            // Suggest-and-confirm, never auto-apply: `N` generates, the pinned
            // rename buffer is what actually sends.
            Action::SuggestName | Action::SuggestNameAll => {
                let bulk = action == Action::SuggestNameAll;
                let purpose = if bulk { Purpose::Bulk } else { Purpose::Offer };
                let keys: Vec<String> = if bulk {
                    // Only the cwd+hash names — a name a human or an earlier
                    // pass chose is not ours to replace in bulk.
                    rows.iter()
                        .filter(|r| r.is_derived_name())
                        .map(Session::key)
                        .collect()
                } else {
                    rows.get(sel)
                        .map(|r| vec![r.key()])
                        .into_iter()
                        .flatten()
                        .collect()
                };
                let (evs, ready) = naming.request(&rows, &keys, purpose);
                for e in evs {
                    log.insert(0, e);
                }
                // Already generated this run (typically by a `Ctrl-N` pass):
                // offer it straight away instead of paying for it twice.
                for key in ready {
                    if let Some(name) = naming.suggestions.get(&key).cloned() {
                        log.insert(
                            0,
                            offer(
                                &key,
                                &name,
                                &rows,
                                &mut renaming,
                                bulk || dash.term.is_some() || tabs.active != Tab::Sessions,
                            ),
                        );
                    }
                }
                log.truncate(EVENT_CAP);
            }
        }
    }
    Ok(())
}

/// Stamp each row with its generated title, so the dashboard draws headlines
/// instead of `app-9d`.
///
/// Rows are rebuilt from Claude's registry on every poll and the registry has no
/// idea what a title is, so this runs per frame rather than per poll — a title
/// that arrives between polls shows up on the next draw, not the next poll.
/// A row keeps whatever title it arrived with when we have none of our own —
/// `discover` never sets one, but a fixture can, and a demo bearclaw that lost its
/// titles to an empty map would draw `app-9d` on every row.
fn apply_titles(rows: &mut [Session], titles: &HashMap<String, String>) {
    for r in rows {
        if let Some(title) = titles.get(&r.key()) {
            r.gen_title = Some(title.clone());
        }
    }
}

/// `ListState::offset` is not bounded by the widget, so a shrinking bearclaw (or a
/// resize) can leave it pointing at nothing and the list renders blank.
fn clamp_offset(dash: &mut Dash, len: usize) {
    let max = len.saturating_sub(1);
    if dash.list.offset() > max {
        *dash.list.offset_mut() = max;
    }
    if let Some(s) = dash.list.selected()
        && s > max
    {
        dash.list.select(Some(max));
    }
}

// --- drawing -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum TermInput {
    None,
    /// Ctrl+L, a tap on the header, or any key once the session has ended.
    Leave,
    Switch(Tab),
    /// Shift+→ (true) / Shift+← (false): the next or previous tab, terminal or not.
    Step(bool),
    /// A mouse drag selected this text; the loop copies it and logs the result.
    Copy(String),
}

fn step(t: Tab, next: bool) -> Tab {
    if next { t.next() } else { t.prev() }
}

/// An event for an open terminal: keys, paste and wheel go into it; the way out and taps on the
/// tab bar come back as what the loop should do.
fn term_input(t: &mut Term, ev: &Event, area: Rect, hits: &[(Tab, u16, u16)]) -> TermInput {
    match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => {
            // Typing moves on from a selection; drop its highlight.
            t.sel.clear();
            if term_keys::is_leave(*k) || !t.embed.alive() {
                return TermInput::Leave;
            }
            // Shift+arrows move between tabs from anywhere; Tab / Shift+Tab stay the program's.
            if let Some(next) = tabs::tab_step(*k).filter(|_| {
                k.modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(k.code, KeyCode::Left | KeyCode::Right)
            }) {
                return TermInput::Step(next);
            }
            if let Some(bytes) = term_keys::encode(*k) {
                t.embed.write(&bytes);
            }
            TermInput::None
        }
        Event::Paste(text) => {
            t.sel.clear();
            t.embed.write(&term_keys::paste(text));
            TermInput::None
        }
        Event::Mouse(m) => match m.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                // Scrolling moves the screen under any selection; drop it, scroll the session.
                t.sel.clear();
                let up = m.kind == MouseEventKind::ScrollUp;
                t.embed.write(&term_keys::wheel(
                    up,
                    m.column.saturating_sub(area.x),
                    m.row.saturating_sub(area.y),
                ));
                TermInput::None
            }
            // A left press on the header or tab bar still leaves / switches; inside the
            // screen it begins a selection (a double-click grabs the word).
            MouseEventKind::Down(MouseButton::Left) => {
                match term_click(*m, area, hits) {
                    TermClick::Leave => return TermInput::Leave,
                    TermClick::Switch(to) => return TermInput::Switch(to),
                    TermClick::None => {}
                }
                if let select::SelAct::Press(cell) = select::sel_action(m, area) {
                    t.sel.press(cell, Instant::now(), t.embed.screen());
                }
                TermInput::None
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let select::SelAct::Drag(cell) = select::sel_action(m, area) {
                    t.sel.drag(cell);
                }
                TermInput::None
            }
            // Release: a real drag (or a double-click word) copies; a plain click clears.
            MouseEventKind::Up(MouseButton::Left) => {
                if matches!(select::sel_action(m, area), select::SelAct::Release)
                    && let Some(range) = t.sel.release()
                {
                    let text = select::text(range, t.embed.screen());
                    if !text.is_empty() {
                        return TermInput::Copy(text);
                    }
                }
                TermInput::None
            }
            _ => TermInput::None,
        },
        _ => TermInput::None,
    }
}

/// Copy selected terminal text to the clipboard and record the outcome. The log isn't
/// on screen while a terminal is open, but the line is there on the way back.
fn copy_selection(text: &str, log: &mut Vec<Ev>) {
    let ev = match clip::copy(text) {
        Ok(sink) => Ev::now(
            "⎘",
            format!("copied {} chars via {}", text.chars().count(), sink.label()),
        ),
        Err(e) => Ev::now("✕", format!("copy failed: {e}")),
    };
    log.insert(0, ev);
    log.truncate(EVENT_CAP);
}

/// Claw's tmux session: prepare its workspace, then reuse the running session or start one.
fn start_claw() -> Receiver<std::result::Result<String, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(find_or_start_claw());
    });
    rx
}

/// Prepare Claw's workspace, then reuse its running session or start one. A `claw` session that
/// isn't ours is left alone.
fn find_or_start_claw() -> std::result::Result<String, String> {
    use crate::core::claw;
    use crate::core::harness::Harness;
    let dir = claw::dir();
    claw::prepare(&dir).map_err(|e| e.to_string())?;
    let target = format!("={}:", claw::SESSION);
    let tmux = |args: &[&str]| {
        crate::core::tools::tmux_cmd()
            .args(args)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    if tmux(&["has-session", "-t", &format!("={}", claw::SESSION)]).is_some() {
        // Ours: marked, or (started before the mark existed) running in the workspace.
        let marked =
            tmux(&["show-options", "-qv", "-t", &target, claw::MARK]).as_deref() == Some("1");
        let here = tmux(&[
            "display-message",
            "-p",
            "-t",
            &target,
            "#{pane_current_path}",
        ]);
        if !marked && here.as_deref().map(std::path::Path::new) != Some(dir.as_path()) {
            return Err(format!(
                "a tmux session called `{}` exists and isn't Claw — rename it, then r",
                claw::SESSION
            ));
        }
        tmux(&["set-option", "-t", &target, claw::MARK, "1"]);
        return Ok(claw::SESSION.to_string());
    }
    let (_, ts) = backend::spawn_tagged(
        crate::core::discovery::Backend::Tmux,
        &dir.display().to_string(),
        backend::Prompt::None,
        None,
        Some(claw::SESSION),
        false,
        &claw::launcher(&Harness::Claude.command()),
        Harness::Claude,
        None,
    )
    .map_err(|e| e.to_string())?;
    let ts = ts.unwrap_or_else(|| claw::SESSION.to_string());
    tmux(&["set-option", "-t", &format!("={ts}:"), claw::MARK, "1"]);
    Ok(ts)
}

/// An open terminal under a one-line header; returns where the screen went (for the wheel).
fn draw_term(f: &mut ratatui::Frame, body: Rect, t: &mut Term, header: String) -> Rect {
    let [head, screen] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
    f.render_widget(
        Paragraph::new(Line::styled(header, Style::default().fg(Color::DarkGray))),
        head,
    );
    t.embed.resize(screen.height, screen.width);
    t.embed.draw(f.buffer_mut(), screen);
    if let Some(range) = t.sel.range() {
        select::highlight(f.buffer_mut(), screen, range);
    }
    screen
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermClick {
    None,
    Leave,
    Switch(Tab),
}

/// A tap while a terminal is open: on its header row (just above `screen`) or on the Sessions
/// label it means "back to the list" — the way out on a phone; another tab switches to it.
/// Taps inside the screen belong to the session.
fn term_click(
    m: crossterm::event::MouseEvent,
    screen: Rect,
    hits: &[(Tab, u16, u16)],
) -> TermClick {
    if m.kind != MouseEventKind::Down(MouseButton::Left) {
        return TermClick::None;
    }
    if m.row + 1 == screen.y {
        return TermClick::Leave;
    }
    if m.row == 0 {
        return match tabs::hit(hits, m.column) {
            Some(Tab::Sessions) => TermClick::Leave,
            Some(t) => TermClick::Switch(t),
            None => TermClick::None,
        };
    }
    TermClick::None
}

/// Enter on a session: one in tmux opens inside the dashboard; anything else (iTerm, or a
/// tmux session that won't attach) gets its own terminal focused, as before.
fn open_session(r: &Session, dash: &mut Dash, log: &mut Vec<Ev>) {
    if r.backend == crate::core::discovery::Backend::Tmux
        && !is_fixture()
        && let Some(ts) = r.tmux_session.clone()
        && open_tmux(&ts, r.session_id.clone(), dash, log)
    {
        return;
    }
    if let Err(e) = backend::focus(r) {
        log.insert(0, Ev::now("✕", format!("focus failed: {e}")));
        log.truncate(EVENT_CAP);
    }
}

/// Attach the dashboard to tmux session `ts` (Claude session `id`, when known). False (with a
/// log line) when it can't.
fn open_tmux(ts: &str, id: Option<String>, dash: &mut Dash, log: &mut Vec<Ev>) -> bool {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    match Embedded::attach(ts, rows.saturating_sub(2), cols) {
        Ok(embed) => {
            dash.term = Some(Term {
                session: ts.to_string(),
                id,
                embed,
                sel: select::Selection::default(),
            });
            true
        }
        Err(e) => {
            log.insert(0, Ev::now("✕", format!("open {ts}: {e}")));
            log.truncate(EVENT_CAP);
            false
        }
    }
}

/// Where `c` offers to start: this host's first spawn dir, else the current directory.
pub(crate) fn default_session_dir() -> String {
    let cfg = crate::core::config::get();
    cfg.spawn_dirs_for(&cfg.self_name())
        .into_iter()
        .next()
        .map(|(_, path)| path)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|d| d.display().to_string())
        })
        .map(|d| crate::core::tools::tildify(&d))
        .unwrap_or_default()
}

/// Start Claude (the configured launcher, with `prompt` as its first one) in a new tmux
/// session in `dir`, off the UI thread.
fn spawn_in_tmux(
    dir: String,
    prompt: Option<String>,
) -> Receiver<std::result::Result<String, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(start_claude(&dir, prompt.as_deref(), None));
    });
    rx
}

/// Start Claude (the configured launcher) in a new tmux session in `dir`, with `prompt` as its
/// first one. `name` names the Claude session and, unless that tmux session is taken (a
/// leftover shell, say), the tmux session too. Returns the tmux session.
pub(crate) fn start_claude(
    dir: &str,
    prompt: Option<&str>,
    name: Option<&str>,
) -> std::result::Result<String, String> {
    use crate::core::harness::Harness;
    if !std::path::Path::new(dir).is_dir() {
        return Err(format!("no such directory: {dir}"));
    }
    let tmux_session = name.map(|n| {
        backend::unique_tmux_name(
            &crate::core::tmux::sanitize_name(n),
            &backend::tmux_session_names(),
        )
    });
    backend::spawn_tagged(
        crate::core::discovery::Backend::Tmux,
        dir,
        prompt.map_or(backend::Prompt::None, backend::Prompt::Inline),
        name,
        tmux_session.as_deref(),
        false,
        &crate::core::tools::without_model_env(&Harness::Claude.command()),
        Harness::Claude,
        None,
    )
    .map_err(|e| e.to_string())
    .and_then(|(_, ts)| ts.ok_or_else(|| "tmux gave no session name".to_string()))
}

/// Row 0 is the tab bar; the rest is the Sessions dashboard or the active tab's view.
#[allow(clippy::too_many_arguments)]
fn draw_app(
    f: &mut ratatui::Frame,
    tabs: &mut Tabs,
    rows: &[Session],
    log: &[Ev],
    sel: usize,
    renaming: Option<&Rename>,
    o: &WatchOpts,
    dash: &mut Dash,
) {
    let [bar, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(f.area());
    let waiting = rows.iter().filter(|r| r.is_waiting()).count();
    let badge = (waiting > 0).then(|| format!("⏸{waiting}"));
    let (line, hits) = tabs::tab_bar(tabs.active, bar.width, badge);
    let used = width_of(
        &line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>(),
    ) as u16;
    f.render_widget(Paragraph::new(line), bar);
    dash.tab_hits = hits;
    dash.refresh_hit = tabs::refresh_button(bar.width, used).map(|(label, x)| {
        let w = width_of(label) as u16;
        f.render_widget(
            Paragraph::new(Line::styled(label, Style::default().fg(Color::Cyan))),
            Rect::new(bar.x + x, bar.y, w, 1),
        );
        (x, x + w)
    });
    if body.height == 0 {
        dash.list_area = Rect::default();
        return;
    }
    if let Some(ed) = dash.editor.as_mut() {
        dash.list_area = Rect::default();
        ed.draw(f, body);
        return;
    }
    if tabs.active == Tab::Claw {
        dash.list_area = Rect::default();
        // Keep the card's PRs/Usage numbers warming while the tab is up (both are stale-guarded).
        for t in [Tab::Prs, Tab::Usage] {
            if let Some(v) = tabs.view(t) {
                v.on_show();
            }
        }
        // A character card beside the terminal on a wide pane; a narrow one keeps all the room.
        let term_body = if body.width >= claw_card::MIN_WIDTH {
            let [card, rest] =
                Layout::horizontal([Constraint::Length(claw_card::WIDTH), Constraint::Min(0)])
                    .areas(body);
            let stale = dash
                .claw_card
                .as_ref()
                .is_none_or(|(at, _)| at.elapsed() >= Duration::from_secs(1));
            if stale {
                let mut stat = tabs::CardStat::default();
                for t in [Tab::Prs, Tab::Usage] {
                    if let Some(v) = tabs.view(t) {
                        v.card_stat(&mut stat);
                    }
                }
                dash.claw_card = Some((Instant::now(), claw_card::Stats::gather(rows, stat)));
            }
            if let Some((_, stats)) = &dash.claw_card {
                claw_card::render(f, card, stats);
            }
            rest
        } else {
            body
        };
        match dash.claw.as_mut() {
            Some(t) => {
                let header = if t.embed.alive() {
                    " ✦ Claw · tell it what to do · ctrl+l (or tap here) back to Sessions ".into()
                } else {
                    " ✦ Claw ended — any key starts it again ".into()
                };
                dash.term_area = draw_term(f, term_body, t, header);
            }
            None => {
                let msg = match &dash.claw_error {
                    Some(e) => format!(" ✦ Claw couldn't start: {e} — r to retry"),
                    None => " ✦ starting Claw…".into(),
                };
                f.render_widget(Paragraph::new(msg).wrap(Wrap { trim: false }), term_body);
            }
        }
        return;
    }
    if tabs.active == Tab::Sessions
        && let Some(t) = dash.term.as_mut()
    {
        dash.list_area = Rect::default();
        let doing =
            t.id.as_ref()
                .and_then(|id| dash.notes.get(id))
                .and_then(|n| n.todo.as_deref())
                .map(|line| format!(" · {line}"))
                .unwrap_or_default();
        let header = if t.embed.alive() {
            format!(
                " ⧉ {}{doing} · ctrl+l (or tap here) back to the list ",
                t.session
            )
        } else {
            format!(" ⧉ {} · the session ended — any key to go back ", t.session)
        };
        dash.term_area = draw_term(f, body, t, header);
        return;
    }
    let active = tabs.active;
    match tabs.view(active) {
        Some(v) => {
            // No session rows on screen: a tap must not reach one.
            dash.list_area = Rect::default();
            v.draw(f, body);
        }
        None => {
            draw_in(f, body, rows, log, sel, renaming, o, dash);
            if let Some(input) = &dash.new_session {
                let line = Rect::new(body.x, body.y, body.width, 1);
                f.render_widget(Clear, line);
                input.draw(f, line);
            }
        }
    }
}

/// The Sessions dashboard filling the whole frame — what the golden tests render.
#[cfg(test)]
fn draw(
    f: &mut ratatui::Frame,
    rows: &[Session],
    log: &[Ev],
    sel: usize,
    renaming: Option<&Rename>,
    o: &WatchOpts,
    dash: &mut Dash,
) {
    let area = f.area();
    draw_in(f, area, rows, log, sel, renaming, o, dash);
}

#[allow(clippy::too_many_arguments)]
fn draw_in(
    f: &mut ratatui::Frame,
    area: Rect,
    rows: &[Session],
    log: &[Ev],
    sel: usize,
    renaming: Option<&Rename>,
    o: &WatchOpts,
    dash: &mut Dash,
) {
    dash.tick = dash.tick.wrapping_add(1);
    let naming = dash.naming;
    let tick = dash.tick;
    // The selected session's note: beside the list on a wide pane, below it on a narrow one
    // (in the events pane's place), and only when there is one.
    let note = rows
        .get(sel)
        .and_then(|r| Some((r, dash.notes.get(r.session_id.as_ref()?)?.clone())));
    let (area, note_area, hide_events) = match &note {
        Some(_) if area.width >= 120 => {
            let [list, side] =
                Layout::horizontal([Constraint::Min(0), Constraint::Length(46)]).areas(area);
            (list, Some(side), false)
        }
        Some((_, n)) => {
            let h = (n.bullets.len() as u16 + 3 + u16::from(n.todo.is_some())).min(9);
            if area.height >= h + 8 {
                let [top, bottom] =
                    Layout::vertical([Constraint::Min(0), Constraint::Length(h)]).areas(area);
                (top, Some(bottom), true)
            } else {
                (area, None, false)
            }
        }
        None => (area, None, false),
    };
    let longest = rows
        .iter()
        .map(|r| width_of(&r.headline()))
        .max()
        .unwrap_or(0);
    let mut plan = dashlayout::plan(
        area.width,
        area.height,
        rows.len(),
        longest,
        dash.force_two_row,
    );

    // `e` overrides the height policy in both directions. Turning events on in a
    // pane too short for both hands them the whole thing below the header.
    let events_height = match dash.events {
        _ if hide_events => 0,
        Some(false) => 0,
        Some(true) if plan.events_height == 0 => area.height.saturating_sub(1),
        _ => plan.events_height,
    };
    let events_height = events_fit(events_height, log.len(), plan.borders);
    let events_height = balance_spare_row(
        events_height,
        area.height,
        plan.borders,
        plan.item_height(),
        rows.len(),
        log.len(),
    );
    plan.events_height = events_height;
    dash.item_height = plan.item_height();
    dash.events_shown = events_height > 0;
    dash.tiny = plan.density == Density::Tiny;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(events_height),
        ])
        .split(area);

    if let Some(r) = renaming {
        // The pinned session can vanish mid-buffer; name it by key rather than
        // silently addressing nobody.
        let who = rows
            .iter()
            .find(|s| s.key() == r.key)
            .map(Session::label)
            .unwrap_or_else(|| short_key(&r.key));
        render_rename(f, chunks[0], &who, &r.buf);
    } else {
        render_header(f, chunks[0], rows, o, &plan, (naming, dash.dormant), tick);
    }
    if let (Some(a), Some((r, n))) = (note_area, &note) {
        render_note(f, a, &r.headline(), n);
    }
    if let Some(mut d) = dash.detail.take() {
        d.draw(f, chunks[1].union(chunks[2]), rows);
        dash.detail = Some(d);
        // No rows on screen: a tap must not focus one.
        dash.list_area = Rect::default();
    } else {
        let ctx = BearclawCtx::of(rows, &plan);
        render_sessions(f, chunks[1], rows, sel, &plan, &ctx, dash);
        if events_height > 0 {
            render_events(f, chunks[2], log, &plan);
        }
    }
    if dash.help {
        render_help(f);
        // The overlay owns every cell it covers: without this a tap on it maps
        // straight through to the row underneath and focuses that terminal.
        dash.list_area = Rect::default();
    }
}

/// The selected session's todo line and sticky bullets, boxed.
fn render_note(f: &mut ratatui::Frame, area: Rect, who: &str, n: &SessionNote) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .title(crate::cli::render::fit(
            &format!(" Sticky · {who} "),
            area.width.saturating_sub(2) as usize,
        ));
    let mut lines: Vec<Line> = Vec::new();
    if let Some(t) = &n.todo {
        lines.push(Line::styled(
            format!("● {t}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    for b in &n.bullets {
        lines.push(Line::raw(format!("• {b}")));
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Trim the events pane to what it actually has to show. An empty log used to
/// hold 8 of a 30-row frame open for one line of "waiting for something to
/// happen…" while the sessions list rendered blank rows; the surplus belongs to
/// the list. Returns 0 when what's left can't fit a single event line.
fn events_fit(height: u16, entries: usize, borders: bool) -> u16 {
    let chrome = if borders { 2 } else { 0 };
    // The placeholder is one line, so an empty log still asks for one.
    let want = (entries.max(1) as u16).saturating_add(chrome);
    let height = height.min(want);
    if height <= chrome { 0 } else { height }
}

/// Hand the sessions list's unusable remainder to the events pane.
///
/// Two-row items can't fill an odd row, so a 16-row phone frame left one blank
/// under the list. A row is worth real money at that size — an extra event line
/// beats a gap. No-op when the events pane is hidden, and stable: the list only
/// ever gives up rows it could not have drawn an item into.
fn balance_spare_row(
    events_height: u16,
    area_h: u16,
    borders: bool,
    item_h: u16,
    rows: usize,
    entries: usize,
) -> u16 {
    let chrome = if borders { 2 } else { 1 };
    // Only worth moving when the events pane has something more to show with it;
    // otherwise the blank row just changes frames.
    let starved = entries as u16 > events_height.saturating_sub(chrome);
    if events_height == 0 || item_h <= 1 || !starved {
        return events_height;
    }
    let inner = area_h
        .saturating_sub(1 + events_height)
        .saturating_sub(chrome);
    let hint = u16::from(
        u16::try_from(rows)
            .unwrap_or(u16::MAX)
            .saturating_mul(item_h)
            > inner,
    );
    let spare = inner.saturating_sub(hint) % item_h;
    events_height.saturating_add(spare)
}

fn render_rename(f: &mut ratatui::Frame, area: Rect, who: &str, buf: &str) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" rename {who} → "),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{buf}▏"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  ⏎ apply · esc cancel",
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        area,
    );
}

/// The header degrades hard: the keybinding list moved into the `?` overlay
/// because at 40 columns it was 150 characters of invisible text, on one
/// un-wrapped line. What's left is emitted in priority order, and any segment
/// that doesn't fit is skipped rather than clipped mid-word.
#[allow(clippy::too_many_arguments)]
fn render_header(
    f: &mut ratatui::Frame,
    area: Rect,
    rows: &[Session],
    o: &WatchOpts,
    plan: &Plan,
    (naming, dormant): (Option<NameProgress>, usize),
    tick: u64,
) {
    let busy = rows.iter().filter(|r| r.status == "busy").count();
    let waiting = rows.iter().filter(|r| r.is_waiting()).count();
    let n = rows.len();
    let wide = plan.density == Density::Wide;
    let tiny = plan.density == Density::Tiny;

    let name = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let body = Style::default().fg(Color::Gray);
    let dim = Style::default().fg(Color::DarkGray);
    let alarm = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let clock = chrono::Local::now();

    // Most important first — a skipped segment doesn't stop the shorter ones
    // behind it from landing.
    let mut segs: Vec<(String, Style)> = vec![
        (" bearclaw ".into(), name),
        if tiny {
            (format!("· {n}·{busy} "), body)
        } else if wide {
            (format!("· {n} live · {busy} working "), body)
        } else {
            (format!("· {n} live · {busy} busy "), body)
        },
    ];
    if waiting > 0 {
        segs.push(if wide {
            (format!("· {waiting} need you "), alarm)
        } else {
            (format!("· {waiting}⏸ "), alarm)
        });
    }
    // A reboot's leftovers, waiting for `bearclaw restore` — easy to miss otherwise.
    if dormant > 0 {
        segs.push(if wide {
            (format!("· {dormant} dormant "), dim)
        } else {
            (format!("· {dormant}◌ "), dim)
        });
    }
    // A model call is seconds long, so the bearclaw has to say it's working on one
    // — otherwise `N` looks like it did nothing at all.
    if let Some(p) = naming {
        let spin = spinner_frame(tick);
        segs.push((
            if wide {
                format!("· {spin} naming {}/{} ", p.done, p.total)
            } else {
                // Seven cells. At 40 columns this costs the `?` hint for as long
                // as the pass runs, which is the right trade: the hint is static,
                // the spinner is the only sign the model is still thinking.
                format!("· {spin}{}/{} ", p.done, p.total)
            },
            Style::default().fg(Color::Cyan),
        ));
    }
    segs.push(if wide {
        ("· ? help ".into(), dim)
    } else {
        ("· ? ".into(), dim)
    });
    segs.push(if wide {
        (format!("· {} ", clock.format("%H:%M:%S")), dim)
    } else {
        (format!("· {} ", clock.format("%H:%M")), dim)
    });
    if wide {
        segs.push((
            format!("· every {}s · stuck>{}s ", o.interval_secs, o.stuck_secs),
            dim,
        ));
    }

    let budget = area.width as usize;
    let mut used = 0;
    let mut spans = Vec::new();
    for (text, style) in segs {
        let w = width_of(&text);
        if used + w > budget {
            continue;
        }
        used += w;
        spans.push(Span::styled(text, style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// ` Sessions · ~/Code/app `: the block's title carries the directory the bearclaw
/// has in common, which is what earns the rows the right to leave it out.
///
/// `BearclawCtx` already dropped the base if this pane couldn't print it, so the two
/// can't disagree — a row never draws a relative path the title failed to explain.
fn sessions_title(ctx: &BearclawCtx) -> String {
    ctx.base
        .as_deref()
        .and_then(rows::base_title)
        .unwrap_or_else(|| " Sessions ".to_string())
}

fn render_sessions(
    f: &mut ratatui::Frame,
    area: Rect,
    rows: &[Session],
    sel: usize,
    plan: &Plan,
    ctx: &BearclawCtx,
    dash: &mut Dash,
) {
    if area.height == 0 || area.width == 0 {
        dash.list_area = Rect::default();
        return;
    }
    // The directory the bearclaw shares is named here, once, instead of on every row.
    let title = sessions_title(ctx);
    let block = if plan.borders {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title)
    } else {
        // No columns to spare for a frame: one title line carries the same info.
        Block::default().title(Span::styled(title, Style::default().fg(Color::DarkGray)))
    };
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        dash.list_area = Rect::default();
        return;
    }

    if rows.is_empty() {
        dash.list_area = Rect::default();
        f.render_widget(
            Paragraph::new(Span::styled(
                "no running claude sessions",
                Style::default().fg(Color::DarkGray),
            )),
            inner,
        );
        return;
    }

    let item_h = plan.item_height();
    // Reserve a line for the overflow hint, the same way a paged list does — and round
    // the list down to whole items, so the hint sits directly under the last one.
    // Left at `inner.height - 1` it drifted a row up or down with the pane's
    // parity: at 44x16 a blank line opened up above it, at 32x12 it didn't.
    let overflows = (rows.len() as u16).saturating_mul(item_h) > inner.height;
    let (list_area, hint_y) = if overflows && inner.height > item_h {
        let height = ((inner.height - 1) / item_h) * item_h;
        (Rect { height, ..inner }, Some(inner.y + height))
    } else {
        (inner, None)
    };

    let items: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| session_item(r, plan, ctx, i, i == sel))
        .collect();
    dash.list.select(Some(sel));
    dash.list_area = list_area;
    f.render_stateful_widget(List::new(items), list_area, &mut dash.list);

    if let Some(y) = hint_y {
        let shown = dash.list.offset() + (list_area.height / item_h.max(1)) as usize;
        let remaining = rows.len().saturating_sub(shown);
        if remaining > 0 {
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!(" +{remaining} more ↓"),
                    Style::default().add_modifier(Modifier::DIM),
                )),
                Rect {
                    y,
                    height: 1,
                    ..inner
                },
            );
        }
    }
}

fn render_events(f: &mut ratatui::Frame, area: Rect, log: &[Ev], plan: &Plan) {
    if area.height == 0 {
        return;
    }
    let ev_lines: Vec<Line> = if log.is_empty() {
        vec![Line::from(Span::styled(
            " waiting for something to happen…",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        log.iter()
            .map(|e| {
                // The icon set is mixed-width — `＋` and `✅` draw two columns,
                // the rest one — so pad to a constant two and keep the times
                // (and everything behind them) in one column.
                let icon = format!(
                    " {}{} ",
                    e.icon,
                    " ".repeat(2usize.saturating_sub(width_of(e.icon)))
                );
                Line::from(vec![
                    Span::raw(icon),
                    Span::styled(format!("{} ", e.time), Style::default().fg(Color::DarkGray)),
                    Span::raw(e.msg.clone()),
                ])
            })
            .collect()
    };
    let block = if plan.borders {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Events ")
    } else {
        Block::default()
    };
    f.render_widget(Paragraph::new(ev_lines).block(block), area);
}

fn render_help(f: &mut ratatui::Frame) {
    const BINDS: [(&str, &str); 16] = [
        ("Tab ⇧Tab", "switch tab (also ⇧← ⇧→, or tap)"),
        ("d →", "open a session: screen, brief, send"),
        ("1-9 0", "jump to that row and focus it"),
        ("⏎ ␣ l o", "open it (tmux: inside here)"),
        ("ctrl+l", "back from an open session"),
        ("c", "start a new session"),
        ("↑↓ jk", "move the selection"),
        ("tap", "select + focus (mouse/touch)"),
        ("n", "rename the selected session"),
        ("r", "refresh now"),
        ("z", "compact 1-line rows (remembered)"),
        ("e", "show/hide the events pane"),
        ("N", "suggest a name for this session"),
        ("^N", "suggest names for unnamed ones"),
        ("?", "toggle this help"),
        ("q Esc", "quit"),
    ];
    let area = centered_rect(48, BINDS.len() as u16 + 2, f.area());
    f.render_widget(Clear, area);
    let lines: Vec<Line> = BINDS
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!(" {k:<9}"), Style::default().fg(Color::Cyan)),
                Span::raw(*v),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(" Keys "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::backend::TestBackend;

    #[test]
    fn detects_awaiting_prompt() {
        assert!(awaiting("│ Do you want to proceed? │"));
        assert!(awaiting("  ❯ 1. Yes  "));
        assert!(!awaiting("✻ Working for 12s"));
    }

    #[test]
    fn row_mode_round_trips_through_the_config_value() {
        for v in [None, Some(true), Some(false)] {
            assert_eq!(resolve_rows(Some(v)), v);
        }
        assert_eq!(rows_setting(None), "auto");
        assert_eq!(rows_setting(Some(true)), "2");
        assert_eq!(rows_setting(Some(false)), "1");
    }

    #[test]
    fn offsets_are_clamped_when_the_bearclaw_shrinks() {
        let mut dash = test_dash();
        *dash.list.offset_mut() = 12;
        dash.list.select(Some(12));
        clamp_offset(&mut dash, 3);
        assert_eq!(dash.list.offset(), 2);
        assert_eq!(dash.list.selected(), Some(2));
        // An empty bearclaw must not underflow.
        clamp_offset(&mut dash, 0);
        assert_eq!(dash.list.offset(), 0);
    }

    fn test_dash() -> Dash {
        Dash {
            list: ListState::default(),
            help: false,
            force_two_row: None,
            events: None,
            list_area: Rect::default(),
            item_height: 1,
            events_shown: false,
            tiny: false,
            naming: None,
            tick: 0,
            dormant: 0,
            tab_hits: Vec::new(),
            refresh_hit: None,
            detail: None,
            editor: None,
            term: None,
            term_area: Rect::default(),
            new_session: None,
            new_prompt: None,
            notes: HashMap::new(),
            claw: None,
            claw_starting: None,
            claw_error: None,
            claw_card: None,
            spawning: None,
        }
    }

    // --- golden buffer -------------------------------------------------------

    fn fixture() -> Vec<Session> {
        let json = r#"[
          {"pid":1,"session_id":"aaaaaaaa-1111","name":"cache-warmup-scheduler",
           "cwd":"/tmp/wt/api-server","status":"busy","tmux_session":"api-server",
           "backend":"tmux","title":"make the cache warmup idempotent so a re-run is free"},
          {"pid":2,"session_id":"bbbbbbbb-2222","name":"docs-refresh",
           "cwd":"/tmp/wt/frontend","status":"waiting","waiting_for":"input needed",
           "tmux_session":"frontend","backend":"tmux","title":"refresh the install docs"},
          {"pid":3,"session_id":"cccccccc-3333","name":"app-9d","name_source":"derived",
           "cwd":"/tmp/app","status":"idle","tab":"app","backend":"iterm",
           "title":"why is the statusline blank"}
        ]"#;
        let mut rows: Vec<Session> = serde_json::from_str(json).unwrap();
        for r in &mut rows {
            r.updated_at = Some(now_ms() - 120_000);
        }
        rows.sort_by_key(|r| !r.is_waiting());
        rows
    }

    /// Every drawn line must fill the frame exactly — measured in cells, since a
    /// rendered double-wide glyph leaves the following cell as a space.
    fn assert_cells(lines: &[String], width: usize) {
        for line in lines {
            assert_eq!(line.chars().count(), width, "{line:?}");
        }
    }

    /// Render the whole dashboard at `w`x`h` and give back the drawn lines.
    fn screen(w: u16, h: u16, rows_mode: Option<bool>) -> Vec<String> {
        let log = vec![Ev::now("▶", "cache-warmup-scheduler working".into())];
        screen_of(w, h, rows_mode, &fixture(), &log)
    }

    fn screen_of(
        w: u16,
        h: u16,
        rows_mode: Option<bool>,
        rows: &[Session],
        log: &[Ev],
    ) -> Vec<String> {
        let o = WatchOpts {
            interval_secs: 5,
            stuck_secs: 300,
            ..Default::default()
        };
        let mut dash = test_dash();
        dash.force_two_row = rows_mode;
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, rows, log, 0, None, &o, &mut dash))
            .unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    fn many(n: usize) -> Vec<Session> {
        (0..n)
            .map(|i| Session {
                name: Some(format!("session-{i:02}")),
                session_id: Some(format!("id-{i}")),
                status: "idle".into(),
                updated_at: Some(now_ms()),
                tmux_session: Some(format!("tmux-{i:02}")),
                ..Default::default()
            })
            .collect()
    }

    // The title is the only thing explaining the rows' relative paths, so it is
    // either drawn whole or not at all — and `BearclawCtx` is where that is decided,
    // so the rows and the title can't disagree.
    #[test]
    fn the_block_title_carries_the_common_base_only_when_it_fits() {
        let at = |dir: &str| Session {
            cwd: Some(dir.to_string()),
            session_id: Some(dir.to_string()),
            status: "idle".into(),
            ..Default::default()
        };
        let home = |rest: &str| format!("{}/{rest}", dirs::home_dir().unwrap().display());
        let title = |w: u16, rows: &[Session]| {
            let plan = dashlayout::plan(w, 40, rows.len(), 22, None);
            sessions_title(&BearclawCtx::of(rows, &plan))
        };

        let shared = [at(&home("Code/app")), at(&home("Code/app/clones/api"))];
        assert_eq!(title(170, &shared), " Sessions · ~/Code/app ");
        // A phone pane names it too: ten columns plus the path is all it takes.
        assert_eq!(title(44, &shared), " Sessions · ~/Code/app ");
        // Nothing in common, nothing to name.
        assert_eq!(title(170, &[at("/tmp/a"), at("/var/b")]), " Sessions ");
        // Too long for the pane: the title stays plain, and the rows then draw
        // whole paths rather than remainders of a base nobody was told about.
        let deep = home("Code/app/branches/a-very-long-worktree-directory-name");
        let long = [at(&deep), at(&deep)];
        assert_eq!(title(40, &long), " Sessions ");
        assert!(title(170, &long).contains("branches"));
    }

    fn app_screen(w: u16, h: u16, tabs: &mut Tabs, rows: &[Session]) -> Vec<String> {
        app_screen_with(w, h, tabs, rows, &mut test_dash())
    }

    fn app_screen_with(
        w: u16,
        h: u16,
        tabs: &mut Tabs,
        rows: &[Session],
        dash: &mut Dash,
    ) -> Vec<String> {
        let o = WatchOpts {
            interval_secs: 5,
            stuck_secs: 300,
            ..Default::default()
        };
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw_app(f, tabs, rows, &[], 0, None, &o, dash))
            .unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    #[test]
    fn shift_arrows_still_switch_tabs_from_an_open_terminal() {
        let mut cmd = portable_pty::CommandBuilder::new("sh");
        cmd.args(["-c", "exec sleep 5"]);
        let mut t = Term {
            session: "x".into(),
            id: None,
            embed: Embedded::attach_cmd(cmd, 5, 20).unwrap(),
            sel: select::Selection::default(),
        };
        let key = |c, m| Event::Key(crossterm::event::KeyEvent::new(c, m));
        let none: &[(Tab, u16, u16)] = &[];
        assert_eq!(
            term_input(
                &mut t,
                &key(KeyCode::Right, KeyModifiers::SHIFT),
                Rect::default(),
                none
            ),
            TermInput::Step(true)
        );
        assert_eq!(
            term_input(
                &mut t,
                &key(KeyCode::Left, KeyModifiers::SHIFT),
                Rect::default(),
                none
            ),
            TermInput::Step(false)
        );
        // Tab and Shift+Tab are Claude's (completion, permission mode).
        assert_eq!(
            term_input(
                &mut t,
                &key(KeyCode::Tab, KeyModifiers::NONE),
                Rect::default(),
                none
            ),
            TermInput::None
        );
        assert_eq!(
            term_input(
                &mut t,
                &key(KeyCode::BackTab, KeyModifiers::SHIFT),
                Rect::default(),
                none
            ),
            TermInput::None
        );
    }

    fn mouse_term(feed: &str) -> Term {
        let mut cmd = portable_pty::CommandBuilder::new("sh");
        cmd.args(["-c", &format!("printf '{feed}'; exec sleep 5")]);
        let mut embed = Embedded::attach_cmd(cmd, 10, 60).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while !embed.parser.screen().contents().contains(feed) && Instant::now() < until {
            embed.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        Term {
            session: "x".into(),
            id: None,
            embed,
            sel: select::Selection::default(),
        }
    }

    fn mouse_ev(kind: crossterm::event::MouseEventKind, row: u16, column: u16) -> Event {
        Event::Mouse(crossterm::event::MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn a_left_drag_in_an_open_terminal_copies_the_selected_text() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut t = mouse_term("hello world");
        let screen = Rect::new(0, 0, 60, 10);
        let none: &[(Tab, u16, u16)] = &[];
        // Press at column 0, drag across "hello", release.
        assert_eq!(
            term_input(
                &mut t,
                &mouse_ev(MouseEventKind::Down(MouseButton::Left), 0, 0),
                screen,
                none
            ),
            TermInput::None
        );
        assert!(t.sel.range().is_some());
        assert_eq!(
            term_input(
                &mut t,
                &mouse_ev(MouseEventKind::Drag(MouseButton::Left), 0, 4),
                screen,
                none
            ),
            TermInput::None
        );
        assert_eq!(
            term_input(
                &mut t,
                &mouse_ev(MouseEventKind::Up(MouseButton::Left), 0, 4),
                screen,
                none
            ),
            TermInput::Copy("hello".into())
        );
        // The highlight persists after the copy, as visible feedback.
        assert!(t.sel.range().is_some());
        // Typing moves on and clears it.
        term_input(
            &mut t,
            &Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::NONE,
            )),
            screen,
            none,
        );
        assert!(t.sel.range().is_none());
    }

    #[test]
    fn a_plain_click_in_an_open_terminal_copies_nothing() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut t = mouse_term("hello world");
        let screen = Rect::new(0, 0, 60, 10);
        let none: &[(Tab, u16, u16)] = &[];
        term_input(
            &mut t,
            &mouse_ev(MouseEventKind::Down(MouseButton::Left), 0, 2),
            screen,
            none,
        );
        assert_eq!(
            term_input(
                &mut t,
                &mouse_ev(MouseEventKind::Up(MouseButton::Left), 0, 2),
                screen,
                none
            ),
            TermInput::None
        );
        assert!(t.sel.range().is_none());
    }

    #[test]
    fn a_double_click_in_an_open_terminal_copies_the_word() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut t = mouse_term("hello world");
        let screen = Rect::new(0, 0, 60, 10);
        let none: &[(Tab, u16, u16)] = &[];
        let down = mouse_ev(MouseEventKind::Down(MouseButton::Left), 0, 1);
        let up = mouse_ev(MouseEventKind::Up(MouseButton::Left), 0, 1);
        // First click: a plain click, nothing selected.
        term_input(&mut t, &down, screen, none);
        assert_eq!(term_input(&mut t, &up, screen, none), TermInput::None);
        // Second click right after, on the same cell: selects and copies the word.
        term_input(&mut t, &down, screen, none);
        assert_eq!(
            term_input(&mut t, &up, screen, none),
            TermInput::Copy("hello".into())
        );
    }

    #[test]
    fn the_wheel_still_scrolls_and_drops_any_selection() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut t = mouse_term("hello world");
        let screen = Rect::new(0, 0, 60, 10);
        let none: &[(Tab, u16, u16)] = &[];
        // Select something first.
        term_input(
            &mut t,
            &mouse_ev(MouseEventKind::Down(MouseButton::Left), 0, 0),
            screen,
            none,
        );
        term_input(
            &mut t,
            &mouse_ev(MouseEventKind::Drag(MouseButton::Left), 0, 4),
            screen,
            none,
        );
        assert!(t.sel.range().is_some());
        // A wheel notch scrolls the session and clears the selection.
        assert_eq!(
            term_input(
                &mut t,
                &mouse_ev(MouseEventKind::ScrollUp, 3, 3),
                screen,
                none
            ),
            TermInput::None
        );
        assert!(t.sel.range().is_none());
    }

    #[test]
    fn taps_leave_an_open_terminal() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let (_, hits) = tabs::tab_bar(Tab::Sessions, 120, None);
        let tap = |column, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let screen = Rect::new(0, 2, 120, 20);
        // The header row above the screen, and the Sessions label itself, both leave.
        assert_eq!(term_click(tap(30, 1), screen, &hits), TermClick::Leave);
        assert_eq!(term_click(tap(2, 0), screen, &hits), TermClick::Leave);
        let &(_, x0, _) = hits.iter().find(|h| h.0 == Tab::Prs).unwrap();
        assert_eq!(
            term_click(tap(x0 + 1, 0), screen, &hits),
            TermClick::Switch(Tab::Prs)
        );
        // Inside the screen a tap belongs to the session.
        assert_eq!(term_click(tap(10, 5), screen, &hits), TermClick::None);
    }

    #[test]
    fn the_claw_tab_says_what_it_is_doing() {
        let rows = fixture();
        let mut tabs = Tabs::new();
        tabs.active = Tab::Claw;
        let s = app_screen(120, 12, &mut tabs, &rows).join("\n");
        assert!(s.contains("starting Claw"), "{s}");
        assert!(!s.contains("3 live"), "{s}");
        let mut dash = test_dash();
        dash.claw_error = Some("tmux: not found".into());
        let s = app_screen_with(120, 12, &mut tabs, &rows, &mut dash).join("\n");
        assert!(
            s.contains("tmux: not found") && s.contains("r to retry"),
            "{s}"
        );
    }

    #[test]
    fn embedded_terminal_fills_the_body() {
        // Still running when drawn: the header offers the way back, not "ended".
        let mut cmd = portable_pty::CommandBuilder::new("sh");
        cmd.args(["-c", "printf 'hello from pty'; exec sleep 5"]);
        let mut embed = Embedded::attach_cmd(cmd, 10, 60).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while !embed.parser.screen().contents().contains("hello from pty") && Instant::now() < until
        {
            embed.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        let rows = fixture();
        let mut tabs = Tabs::new();
        let mut dash = test_dash();
        dash.term = Some(Term {
            session: "demo".into(),
            id: None,
            embed,
            sel: select::Selection::default(),
        });
        let s = app_screen_with(120, 20, &mut tabs, &rows, &mut dash);
        assert!(s[0].contains("Sessions"), "{:?}", s[0]);
        assert!(
            s[1].contains("demo") && s[1].contains("ctrl+l"),
            "{:?}",
            s[1]
        );
        assert!(s[2].contains("hello from pty"), "{s:?}");
        assert!(!s.join("\n").contains("3 live"), "{s:?}");
    }

    #[test]
    fn golden_tab_bar_on_top() {
        let rows = fixture();
        let mut tabs = Tabs::new();
        let s = app_screen(120, 30, &mut tabs, &rows);
        assert!(
            s[0].contains("Sessions") && s[0].contains("Settings"),
            "{:?}",
            s[0]
        );
        // The waiting session is counted on the Sessions tab, visible from every tab.
        assert!(s[0].contains("Sessions ⏸1"), "{:?}", s[0]);
        assert!(s[1].contains("3 live"), "{:?}", s[1]);
        // `active`, not `switch`: showing a tab starts its fetch, and a test must not call GitHub.
        tabs.active = Tab::Prs;
        let s = app_screen(120, 30, &mut tabs, &rows);
        assert!(s[0].contains("Sessions ⏸1"), "{:?}", s[0]);
        assert!(s[1].contains(" PRs "), "{:?}", s[1]);
        assert!(!s.join("\n").contains("3 live"), "{s:?}");
    }

    fn noted_dash() -> Dash {
        let mut dash = test_dash();
        dash.notes.insert(
            "bbbbbbbb-2222".into(),
            SessionNote {
                todo: Some("Permission error in the loader".into()),
                bullets: vec!["Found the 403".into(), "Fixed the role hash".into()],
            },
        );
        dash
    }

    fn screen_dash(w: u16, h: u16, rows: &[Session], dash: &mut Dash) -> Vec<String> {
        let o = WatchOpts {
            interval_secs: 5,
            stuck_secs: 300,
            ..Default::default()
        };
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, rows, &[], 0, None, &o, dash)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    #[test]
    fn the_sticky_note_sits_beside_the_list_on_a_wide_pane() {
        let rows = fixture();
        let s = screen_dash(150, 30, &rows, &mut noted_dash());
        let line = s
            .iter()
            .find(|l| l.contains("Found the 403"))
            .expect("note drawn");
        assert!(line.find("Found the 403").unwrap() > 100, "{line}");
        let all = s.join("\n");
        assert!(
            all.contains("Permission error in the loader") && all.contains("docs-refresh"),
            "{all}"
        );
        // Nothing noted, nothing taken from the list.
        let plain = screen_dash(150, 30, &rows, &mut test_dash()).join("\n");
        assert!(!plain.contains("Sticky"), "{plain}");
    }

    #[test]
    fn on_a_narrow_pane_the_note_goes_below_the_list() {
        let rows = fixture();
        let s = screen_dash(80, 30, &rows, &mut noted_dash());
        let all = s.join("\n");
        assert!(
            all.contains("Found the 403") && all.contains("docs-refresh"),
            "{all}"
        );
        let note_row = s.iter().position(|l| l.contains("Found the 403")).unwrap();
        let list_row = s.iter().position(|l| l.contains("docs-refresh")).unwrap();
        assert!(note_row > list_row, "{all}");
    }

    #[test]
    fn golden_desktop() {
        // 170 columns is a common wide terminal to watch the bearclaw in.
        let s = screen(170, 40, None);
        let all = s.join("\n");
        // Header: counts, and the waiting session called out.
        assert!(s[0].contains("bearclaw"), "{:?}", s[0]);
        assert!(s[0].contains("3 live"), "{:?}", s[0]);
        assert!(s[0].contains("1 need you"), "{:?}", s[0]);
        assert!(s[0].contains("? help"), "{:?}", s[0]);
        // The directory the bearclaw shares is named once, in the block's title —
        // which is what buys the rows the right to leave it off.
        assert!(s[1].contains("Sessions · /tmp/wt"), "{:?}", s[1]);
        assert!(all.contains("Events"), "{all}");
        // Two lines per session: identity on the first, the prompt on the second.
        assert!(s[2].contains("docs-refresh"), "{:?}", s[2]);
        assert!(s[2].contains("needs you"), "{:?}", s[2]);
        assert!(s[2].contains("⧉ frontend"), "{:?}", s[2]);
        assert!(s[3].contains("refresh the install docs"), "{:?}", s[3]);
        assert!(s[3].trim_matches(['│', ' ']).len() > 10, "{:?}", s[3]);
        // Full names, and the prompt no longer competes for the same line.
        assert!(s[4].contains("cache-warmup-scheduler"), "{:?}", s[4]);
        assert!(
            s[5].contains("make the cache warmup idempotent so a re-run is free"),
            "{:?}",
            s[5]
        );
        // iTerm sessions fall back to their tab title, and the one session that
        // isn't under the common base draws its whole path.
        assert!(all.contains("▣ app"), "{all}");
        assert!(all.contains("/tmp/app"), "{all}");
        assert_cells(&s, 170);
    }

    // The same shape has to hold across the widths a desktop terminal actually
    // gets resized to, borders and all.
    #[test]
    fn golden_desktop_widths_all_draw_two_line_items() {
        for (w, h) in [(160u16, 40u16), (200, 50), (120, 40), (100, 30)] {
            let s = screen(w, h, None);
            let all = s.join("\n");
            assert!(s[1].contains("Sessions"), "{w}x{h}: {:?}", s[1]);
            // Row 1 is the hoisted waiting session; row 2 is its prompt.
            assert!(s[2].contains("docs-refresh"), "{w}x{h}: {:?}", s[2]);
            assert!(s[2].contains("needs you"), "{w}x{h}: {:?}", s[2]);
            assert!(
                s[3].contains("refresh the install docs"),
                "{w}x{h}: {:?}",
                s[3]
            );
            // …and the next session starts two lines down, numbered 2.
            assert!(s[4].contains(" 2 "), "{w}x{h}: {:?}", s[4]);
            assert!(
                s[4].contains("cache-warmup-scheduler"),
                "{w}x{h}: {:?}",
                s[4]
            );
            assert!(all.contains("⧉ api-server"), "{w}x{h}: {all}");
            assert_cells(&s, w as usize);
        }
    }

    // `z`: one line per session, for when fifteen of them matter more than
    // reading any one of them.
    #[test]
    fn golden_desktop_compact() {
        let s = screen(170, 40, Some(false));
        let all = s.join("\n");
        // Three sessions, three consecutive rows.
        assert!(s[2].contains("docs-refresh"), "{:?}", s[2]);
        assert!(s[3].contains("cache-warmup-scheduler"), "{:?}", s[3]);
        assert!(s[4].contains("why-is-the"), "{:?}", s[4]);
        // Still numbered: the digits are how a phone client acts on a row.
        assert!(s[2].contains("▸1"), "{:?}", s[2]);
        // The prompt shares the line again, but the repeated base directory does
        // not — it stays in the title.
        assert!(s[3].contains("make the cache warmup"), "{:?}", s[3]);
        assert!(!s[3].contains("/tmp/wt/api-server"), "{:?}", s[3]);
        assert!(all.contains("Sessions · /tmp/wt"), "{all}");
        assert_cells(&s, 170);
    }

    #[test]
    fn golden_phone() {
        let s = screen(50, 20, None);
        let all = s.join("\n");
        // Three lines, one job each: identity, terminal, prompt.
        assert!(s[2].contains("▸1"), "{:?}", s[2]);
        assert!(s[2].contains("docs-refresh"), "{:?}", s[2]);
        assert!(s[2].contains("needs you"), "{:?}", s[2]);
        assert!(s[3].contains("⧉ frontend"), "{:?}", s[3]);
        assert!(s[4].contains("refresh the install docs"), "{:?}", s[4]);
        // A phone pane names the shared directory too, so the rows stop repeating
        // it — and the one session outside it still draws its path.
        assert!(s[1].contains("Sessions · /tmp/wt"), "{:?}", s[1]);
        assert!(!s[3].contains("/tmp"), "{:?}", s[3]);
        assert!(s[9].contains("/tmp/app"), "{:?}", s[9]);
        // Second session starts three lines down, numbered 2.
        assert!(s[5].contains(" 2"), "{:?}", s[5]);
        assert!(s[5].contains("cache-warmup-scheduler"), "{:?}", s[5]);
        // The prompt gets the whole line: behind the terminal label it used to be
        // a ~15-column stub on a 50-column pane.
        assert!(
            s[7].contains("make the cache warmup idempotent so a re-r"),
            "{:?}",
            s[7]
        );
        // Header drops the interval/stuck detail but keeps the counts.
        assert!(s[0].contains("3 live"), "{:?}", s[0]);
        assert!(!s[0].contains("stuck>"), "{:?}", s[0]);
        // Events still fit at 20 rows.
        assert!(all.contains("Events"), "{all}");
        assert_cells(&s, 50);
    }

    #[test]
    fn golden_phone_split() {
        let s = screen(40, 16, None);
        let all = s.join("\n");
        // Borderless at 40 columns — the frame's two columns go to the content.
        assert!(!s[1].starts_with('╭'), "{:?}", s[1]);
        assert!(s[1].contains("Sessions"), "{:?}", s[1]);
        assert!(s[2].contains("docs-refresh"), "{:?}", s[2]);
        assert!(s[3].contains("⧉ frontend"), "{:?}", s[3]);
        assert!(s[4].contains("refresh the install docs"), "{:?}", s[4]);
        assert!(all.contains("cache-warmup-schedu"), "{all}");
        assert_cells(&s, 40);
    }

    // 32 columns: line 1 hands the status word to line 2 so the name keeps its
    // room, and the third session still has to be on screen.
    #[test]
    fn golden_phone_narrow_split() {
        let s = screen(32, 12, None);
        assert!(s[2].contains("docs-refresh"), "{:?}", s[2]);
        assert!(!s[2].contains("needs you"), "{:?}", s[2]);
        assert!(s[3].contains("needs you"), "{:?}", s[3]);
        assert!(s[3].contains("⧉ frontend"), "{:?}", s[3]);
        assert!(s[4].contains("refresh the install docs"), "{:?}", s[4]);
        // Three items in twelve rows — the events pane gives the row up.
        assert!(s[8].contains("why-is"), "{:?}", s);
        assert_cells(&s, 32);
    }

    #[test]
    fn golden_forced_one_row_on_a_phone() {
        let s = screen(50, 20, Some(false));
        // One line per session, so all three land on consecutive rows.
        assert!(s[2].contains("docs-refresh"), "{:?}", s[2]);
        assert!(s[3].contains("cache-warmup"), "{:?}", s[3]);
        assert!(s[4].contains("why-is-the"), "{:?}", s[4]);
        assert_cells(&s, 50);
    }

    #[test]
    fn a_short_pane_hides_the_events_and_still_lists_sessions() {
        let s = screen(60, 9, None);
        let all = s.join("\n");
        assert!(!all.contains("Events"), "{all}");
        assert!(all.contains("docs-refresh"), "{all}");
    }

    // 15 sessions in a short pane: the selection has to stay on screen and the
    // hidden remainder has to be advertised.
    #[test]
    fn overflow_is_advertised() {
        let mut rows = Vec::new();
        for i in 0..15 {
            let mut s = Session {
                name: Some(format!("session-{i:02}")),
                status: "idle".into(),
                ..Default::default()
            };
            s.session_id = Some(format!("id-{i}"));
            s.updated_at = Some(now_ms());
            rows.push(s);
        }
        let o = WatchOpts::default();
        let mut dash = test_dash();
        let mut t = Terminal::new(TestBackend::new(60, 12)).unwrap();
        t.draw(|f| draw(f, &rows, &[], 0, None, &o, &mut dash))
            .unwrap();
        let buf = t.backend().buffer().clone();
        let all: String = (0..12)
            .map(|y| (0..60u16).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("more ↓"), "{all}");
    }

    // --- rename buffer -------------------------------------------------------

    /// The rename is aimed at a session key, so a poll that re-sorts (or drops)
    /// the rows under the open buffer can't redirect it at whoever is on top.
    #[test]
    fn a_rename_resolves_the_session_it_was_pinned_to() {
        let mut rows = fixture();
        let target = rows[2].key();
        let label = rows[2].label();
        let r = Rename {
            key: target,
            buf: "  fresh-name  ".into(),
        };
        // Re-sort under the buffer: row 0 is now someone else entirely.
        rows.reverse();
        let (_, msg, _) = commit_rename(&r, &rows, false).expect("a non-empty buffer acts");
        // These fixture sessions have no controllable handle, so the send fails —
        // but it fails naming the *pinned* session, which is the point.
        assert!(msg.contains(&label), "{msg}");
    }

    #[test]
    fn a_rename_whose_session_vanished_is_dropped_not_redirected() {
        let rows = fixture();
        let r = Rename {
            key: "no-such-session".into(),
            buf: "fresh-name".into(),
        };
        let (icon, msg, _) =
            commit_rename(&r, &rows, false).expect("a vanished target is worth saying");
        assert_eq!(icon, "✕");
        assert!(msg.contains("gone"), "{msg}");
        // Nobody else was renamed in its place.
        for s in &rows {
            assert!(!msg.contains(&s.label()), "{msg}");
        }
    }

    // The CLI verb validates and crops what it is given; the typed buffer never
    // did, so `/rename <300 chars>` went into a live TUI and came back out as a
    // 300-character tmux session name.
    #[test]
    fn a_typed_name_is_capped_like_the_cli_verb_caps_it() {
        let rows = fixture();
        let r = Rename {
            key: rows[0].key(),
            buf: "z".repeat(300),
        };
        let (_, msg, _) = commit_rename(&r, &rows, false).expect("a non-empty buffer acts");
        let longest = msg
            .split_whitespace()
            .map(|w| w.chars().count())
            .max()
            .unwrap_or(0);
        assert!(
            longest <= commands::MAX_RENAME,
            "an uncapped {longest}-character name went out: {msg}"
        );
    }

    // `N`/`^N` are what the naming feature is; listing them below "quit" reads
    // as an afterthought.
    #[test]
    fn the_help_overlay_lists_the_naming_keys_before_quit() {
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(render_help).unwrap();
        let buf = t.backend().buffer().clone();
        let text: Vec<String> = (0..24)
            .map(|y| (0..80u16).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect();
        let row = |needle: &str| {
            text.iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle} is not in the overlay:\n{}", text.join("\n")))
        };
        assert!(row("suggest a name") < row("quit"));
        assert!(row("suggest names for unnamed") < row("quit"));
    }

    #[test]
    fn an_empty_rename_buffer_does_nothing() {
        let r = Rename {
            key: fixture()[0].key(),
            buf: "   ".into(),
        };
        assert!(commit_rename(&r, &fixture(), false).is_none());
    }

    // Mouse capture is on by default, so a stray tap arrives mid-rename. It must
    // not reach `mouse_action`: that moved the selection *and* focused another
    // terminal, and the pending `/rename` then went to that session.
    #[test]
    fn the_rename_buffer_swallows_mouse_events() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let at = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                column: 5,
                row: 4,
                modifiers: KeyModifiers::NONE,
            })
        };
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::ScrollUp,
            MouseEventKind::ScrollDown,
        ] {
            assert_eq!(rename_event(&at(kind)), RenameAction::None);
        }
        // Keys still get through — including Termius' LF-as-Return.
        assert_eq!(
            rename_event(&Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::CONTROL
            ))),
            RenameAction::Commit
        );
    }

    // --- naming --------------------------------------------------------------

    fn naming_with(enabled: bool, auto: bool) -> Naming {
        Naming {
            pool: None,
            titles: HashMap::new(),
            titled_from: HashMap::new(),
            suggestions: HashMap::new(),
            queued: HashSet::new(),
            wanted: HashMap::new(),
            exhausted: HashSet::new(),
            pending: 0,
            total: 0,
            enabled,
            auto,
            model: "haiku".into(),
            // Never the real file: a unit test must not read (or seed itself
            // from) whatever this machine's bearclaw happens to be called.
            disk: NameCache::at(std::path::PathBuf::new()),
        }
    }

    fn naming_off() -> Naming {
        naming_with(false, true)
    }

    // --- titles --------------------------------------------------------------

    // The point of the whole feature: a row's first line is the generated title,
    // not the Claude session name (`app-9d`) and not the terminal tab name.
    #[test]
    fn a_title_replaces_the_session_name_on_the_row() {
        let mut rows = fixture();
        let derived = rows.iter().position(Session::is_derived_name).unwrap();
        let key = rows[derived].key();
        // The row as it stands: a derived name and an iTerm tab called `app`.
        assert_eq!(rows[derived].tab.as_deref(), Some("app"));
        let titles = HashMap::from([(key, "statusline-blank".to_string())]);
        apply_titles(&mut rows, &titles);

        let drawn = screen_of(120, 20, None, &rows, &[]).join("\n");
        assert!(drawn.contains("statusline-blank"), "{drawn}");
        assert!(!drawn.contains("app-9d"), "{drawn}");
    }

    // …and until one exists the row still has to say *something*: the
    // heuristic slug of the first prompt (`display_title`), not `app-9d`.
    #[test]
    fn an_untitled_row_falls_back_to_the_heuristic_title() {
        let mut rows = fixture();
        apply_titles(&mut rows, &HashMap::new());
        let drawn = screen_of(120, 20, None, &rows, &[]).join("\n");
        assert!(drawn.contains("why-is-the-statusline-blank"), "{drawn}");
    }

    // Titling is automatic, so the pass has to be a no-op in the steady state:
    // twelve `claude` children per poll is not a background feature.
    #[test]
    fn the_titling_pass_asks_for_each_session_once() {
        let rows = fixture();
        let mut n = naming_with(true, true);
        assert_eq!(n.due_for_title(&rows).len(), rows.len());

        for s in &rows {
            n.titled_from.insert(s.key(), s.title.clone());
        }
        assert!(n.due_for_title(&rows).is_empty());
    }

    // A session shows up in the registry before its first prompt is in the
    // transcript, so the title it gets at boot is made from a directory and not
    // much else. When the prompt lands, that title is owed a second look.
    #[test]
    fn a_prompt_that_lands_later_re_opens_the_title() {
        let mut rows = fixture();
        let mut n = naming_with(true, true);
        rows[0].title = None;
        n.titled_from.insert(rows[0].key(), None);
        for s in rows.iter().skip(1) {
            n.titled_from.insert(s.key(), s.title.clone());
        }
        assert!(n.due_for_title(&rows).is_empty());

        rows[0].title = Some("profile the search index re-render storm".into());
        assert_eq!(n.due_for_title(&rows), vec![rows[0].key()]);
    }

    // In flight and already fell back are both "leave it alone": the first would
    // pay twice, the second re-runs a call that is failing today.
    #[test]
    fn the_titling_pass_skips_queued_and_exhausted_sessions() {
        let rows = fixture();
        let mut n = naming_with(true, true);
        n.queued.insert(rows[0].key());
        n.exhausted.insert(rows[1].key());
        assert_eq!(n.due_for_title(&rows), vec![rows[2].key()]);
    }

    // `[naming] auto_title = false` (and fixture mode) start no pool at all.
    #[test]
    fn auto_title_off_starts_nothing() {
        let rows = fixture();
        for mut n in [naming_with(true, false), naming_with(false, true)] {
            n.autotitle(&rows);
            assert!(n.pool.is_none());
            assert_eq!(n.pending, 0);
            assert!(n.titles.is_empty());
        }
    }

    // A background title is display state: it must not open a rename buffer, and
    // it must not put a line in the event log for every session in the bearclaw.
    #[test]
    fn a_background_title_neither_offers_nor_logs() {
        let mut n = naming_with(true, true);
        let key = fixture()[0].key();
        let purpose = n.purpose_of(&key, Purpose::Title);
        assert_eq!(purpose, Purpose::Title);
        assert!(!purpose.is_interactive());
    }

    // …unless the user pressed `N` while it was in flight. The job in the channel
    // can't be amended, so the *answer* is upgraded instead.
    #[test]
    fn n_pressed_during_a_titling_pass_still_offers() {
        let rows = fixture();
        let mut n = naming_with(true, true);
        let key = rows[0].key();
        n.queued.insert(key.clone());

        let (evs, ready) = n.request(&rows, std::slice::from_ref(&key), Purpose::Offer);
        // Nothing new was queued and nothing was ready — but the want was recorded.
        assert!(ready.is_empty());
        assert!(evs.is_empty(), "{}", evs.len());
        assert_eq!(n.purpose_of(&key, Purpose::Title), Purpose::Offer);
        // Consumed: the next answer for that key is a title again.
        assert_eq!(n.purpose_of(&key, Purpose::Title), Purpose::Title);
    }

    // A title already in hand is what `N` offers, so the ordinary keypress never
    // queues behind the titling backlog and never pays for the same name twice.
    #[test]
    fn n_offers_the_title_the_row_is_already_showing() {
        let rows = fixture();
        let mut n = naming_with(true, true);
        let key = rows[0].key();
        n.titles.insert(key.clone(), "upload-retry-limit".into());

        let (_, ready) = n.request(&rows, std::slice::from_ref(&key), Purpose::Offer);
        assert_eq!(ready, vec![key.clone()]);
        assert!(n.pool.is_none());
        assert_eq!(
            n.suggestions.get(&key).map(String::as_str),
            Some("upload-retry-limit")
        );
    }

    // Fixture mode (and `[naming] enabled = false`) must never reach the model:
    // no pool, no child, one honest line in the log.
    #[test]
    fn naming_disabled_says_so_and_starts_nothing() {
        let rows = fixture();
        let mut n = naming_off();
        let (evs, ready) = n.request(&rows, &[rows[0].key()], Purpose::Offer);
        assert!(n.pool.is_none());
        assert_eq!(n.pending, 0);
        assert!(ready.is_empty());
        assert!(evs[0].msg.contains("naming is off"), "{}", evs[0].msg);
    }

    // A generated name is a *suggestion*: it prefills the pinned rename buffer
    // and waits for ⏎. Nothing is sent by the act of generating it.
    #[test]
    fn a_suggestion_prefills_the_rename_buffer_rather_than_sending() {
        let rows = fixture();
        // app-9d is the idle, derived-name row.
        let target = rows.iter().find(|r| r.is_derived_name()).unwrap().key();
        let mut renaming = None;
        let ev = offer(&target, "statusline-blank", &rows, &mut renaming, false);
        let r = renaming.expect("the buffer opens");
        assert_eq!(r.key, target);
        assert_eq!(r.buf, "statusline-blank");
        assert!(ev.msg.contains("⏎ applies"), "{}", ev.msg);
    }

    // `backend::send` types into a live Claude TUI. A session sitting on a
    // permission prompt would read `/rename foo` as its answer; a busy one runs
    // it mid-turn as a local command (core::title::Hold), so it gets the buffer.
    #[test]
    fn a_waiting_session_is_held_a_busy_one_is_prompted() {
        let rows = fixture();
        let s = rows.iter().find(|r| r.status == "waiting").unwrap();
        let mut renaming = None;
        let ev = offer(&s.key(), "some-name", &rows, &mut renaming, false);
        assert!(renaming.is_none(), "waiting opened a buffer");
        assert!(ev.msg.contains("held"), "{}", ev.msg);

        let s = rows.iter().find(|r| r.status == "busy").unwrap();
        let mut renaming = None;
        offer(&s.key(), "some-name", &rows, &mut renaming, false);
        assert!(renaming.is_some(), "busy is renameable");
    }

    // The job outlives the row it was started on. It must be dropped, never
    // re-aimed at whoever drifted into that slot.
    #[test]
    fn a_suggestion_for_a_vanished_session_is_dropped() {
        let rows = fixture();
        let mut renaming = None;
        let ev = offer("no-such-session", "orphan", &rows, &mut renaming, false);
        assert!(renaming.is_none());
        assert!(ev.msg.contains("gone"), "{}", ev.msg);
        for s in &rows {
            assert!(!ev.msg.contains(&s.label()), "{}", ev.msg);
        }
    }

    // A bulk pass reports into the event log; twelve rename buffers in a row is
    // not a confirmation flow.
    #[test]
    fn a_bulk_result_logs_instead_of_opening_a_buffer() {
        let rows = fixture();
        let target = rows.iter().find(|r| r.is_derived_name()).unwrap().key();
        let mut renaming = None;
        let ev = offer(&target, "statusline-blank", &rows, &mut renaming, true);
        assert!(renaming.is_none());
        assert!(ev.msg.contains("press N to apply"), "{}", ev.msg);
    }

    // …and an open buffer is whatever the user is typing right now: a result
    // arriving mid-edit must not overwrite it.
    #[test]
    fn a_result_never_clobbers_an_open_rename_buffer() {
        let rows = fixture();
        let target = rows.iter().find(|r| r.is_derived_name()).unwrap().key();
        let mut renaming = Some(Rename {
            key: rows[0].key(),
            buf: "half-typed".into(),
        });
        offer(&target, "statusline-blank", &rows, &mut renaming, false);
        let r = renaming.unwrap();
        assert_eq!(r.buf, "half-typed");
        assert_eq!(r.key, rows[0].key());
    }

    #[test]
    fn progress_counts_answers_not_jobs() {
        let mut n = naming_off();
        n.pending = 3;
        n.total = 3;
        assert_eq!(n.progress(), Some(NameProgress { done: 0, total: 3 }));
        n.settle("a", false);
        assert_eq!(n.progress(), Some(NameProgress { done: 1, total: 3 }));
        n.settle("b", true);
        n.settle("c", false);
        // Back to idle: the counter resets so the next pass starts from 0.
        assert_eq!(n.progress(), None);
        assert_eq!(n.total, 0);
        // The one that fell back is not retried this run.
        assert!(n.exhausted.contains("b"));
        assert!(!n.exhausted.contains("a"));
        // …and it can't underflow if a stray message arrives late.
        n.settle("d", false);
        assert_eq!(n.pending, 0);
    }

    // The spinner and the count have to be visible on a phone-sized header, not
    // just a 120-column one.
    #[test]
    fn the_header_shows_naming_progress() {
        let rows = fixture();
        let o = WatchOpts::default();
        for (w, h) in [(120u16, 40u16), (50, 20), (40, 16)] {
            let mut dash = test_dash();
            dash.naming = Some(NameProgress { done: 1, total: 3 });
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| draw(f, &rows, &[], 0, None, &o, &mut dash))
                .unwrap();
            let buf = t.backend().buffer().clone();
            let head: String = (0..w).map(|x| buf[(x, 0)].symbol()).collect();
            assert!(head.contains("1/3"), "{w}x{h}: {head:?}");
        }
    }

    // --- small hazards -------------------------------------------------------

    #[test]
    fn short_key_counts_characters_not_bytes() {
        // A fixture id is user-supplied JSON; byte-slicing this panics.
        assert_eq!(short_key("čćžšđčćžšđ"), "čćžšđčćž");
        assert_eq!(short_key("abc"), "abc");
    }

    #[test]
    fn poll_again_in_survives_a_freshly_booted_machine() {
        // `Instant::now() - interval` aborts the TUI here.
        let far = Duration::from_secs(60 * 60 * 24 * 365 * 100);
        let _ = poll_again_in(far, Duration::from_secs(2));
        let t = poll_again_in(Duration::from_secs(30), Duration::from_secs(2));
        assert!(t <= Instant::now());
    }

    // --- vertical space ------------------------------------------------------

    #[test]
    fn an_empty_event_log_does_not_reserve_the_pane() {
        // 3 lines with borders (one line + the frame), not the policy's 10.
        assert_eq!(events_fit(10, 0, true), 3);
        assert_eq!(events_fit(10, 1, true), 3);
        assert_eq!(events_fit(10, 20, true), 10);
        assert_eq!(events_fit(5, 0, false), 1);
        // Nothing to draw into is nothing worth reserving.
        assert_eq!(events_fit(2, 5, true), 0);
        assert_eq!(events_fit(0, 5, false), 0);
    }

    #[test]
    fn the_surplus_of_an_empty_log_goes_to_the_sessions_list() {
        let rows = many(15);
        let s = screen_of(120, 30, None, &rows, &[]);
        let all = s.join("\n");
        assert!(all.contains("waiting for something to happen"), "{all}");
        // The events pane holds one line plus its frame, so the list gets the
        // other 24 rows — eleven two-line items and the overflow hint, not the
        // eight a 10-row events pane would have left room for.
        assert!(all.contains("session-10"), "{all}");
        assert!(all.contains("more ↓"), "{all}");
        // One line per session and the whole bearclaw fits with room to spare.
        let s = screen_of(120, 30, Some(false), &rows, &[]);
        let all = s.join("\n");
        assert!(all.contains("session-14"), "{all}");
        assert!(!all.contains("more ↓"), "{all}");
    }

    // The hint used to float a row above the list at 44x16 but not at 32x12,
    // purely from the pane's parity.
    #[test]
    fn the_overflow_hint_hugs_the_list() {
        let rows = many(15);
        for (w, h) in [(44u16, 16u16), (32, 12), (50, 20), (40, 18)] {
            let s = screen_of(w, h, None, &rows, &[Ev::now("▶", "x".into())]);
            let i = s
                .iter()
                .position(|l| l.contains("more ↓"))
                .unwrap_or_else(|| panic!("{w}x{h} lost the overflow hint:\n{}", s.join("\n")));
            let above = &s[i - 1];
            assert!(
                !above.trim_matches(['│', ' ']).is_empty(),
                "{w}x{h}: blank row above the hint:\n{}",
                s.join("\n")
            );
        }
    }

    #[test]
    fn a_spare_row_under_two_row_items_goes_to_the_events_pane() {
        // 16 rows: header 1 + events 3 leaves 12, i.e. 10 inside the borders —
        // 4 items and the hint use 9, so the odd row is handed to the events.
        assert_eq!(balance_spare_row(3, 16, true, 2, 15, 8), 4);
        // Nothing to fill it with: the row stays where it is.
        assert_eq!(balance_spare_row(3, 16, true, 2, 15, 1), 3);
        // One-row items use every row, and a hidden pane stays hidden.
        assert_eq!(balance_spare_row(3, 16, true, 1, 15, 8), 3);
        assert_eq!(balance_spare_row(0, 16, true, 2, 15, 8), 0);
    }

    // `＋` and `✅` are two columns wide, the rest of the icon set is one; the
    // times behind them used to sit a column apart.
    #[test]
    fn event_icons_of_mixed_width_line_up() {
        let log = vec![
            Ev::now("＋", "a appeared".into()),
            Ev::now("✕", "b closed".into()),
            Ev::now("✅", "c finished".into()),
            Ev::now("⏸", "d needs you".into()),
        ];
        let s = screen_of(120, 40, None, &fixture(), &log);
        // Every event line starts its timestamp in the same column, whatever the
        // icon's width. Measured in cells: a wide glyph blanks the cell after it.
        let times: Vec<usize> = s
            .iter()
            .filter(|l| {
                ["a appeared", "b closed", "c finished", "d needs you"]
                    .iter()
                    .any(|m| l.contains(m))
            })
            .map(|l| l.chars().position(|c| c.is_ascii_digit()).unwrap())
            .collect();
        assert_eq!(times.len(), 4, "{s:?}");
        assert!(times.iter().all(|c| *c == times[0]), "{times:?}\n{s:?}");
    }

    // The overlay owns its cells: `dash.list_area` has to be empty while it's up,
    // or a tap on the panel selects and focuses the row behind it.
    #[test]
    fn the_help_overlay_swallows_taps() {
        let rows = fixture();
        let o = WatchOpts::default();
        let mut dash = test_dash();
        dash.help = true;
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| draw(f, &rows, &[], 0, None, &o, &mut dash))
            .unwrap();
        assert_eq!(dash.list_area, Rect::default());
        assert_eq!(
            keys::mouse_action(
                crossterm::event::MouseEvent {
                    kind: crossterm::event::MouseEventKind::Down(
                        crossterm::event::MouseButton::Left
                    ),
                    column: 40,
                    row: 12,
                    modifiers: KeyModifiers::NONE,
                },
                dash.list_area,
                0,
                1
            ),
            Action::None
        );

        // With the overlay down the same tap lands on a row again.
        dash.help = false;
        t.draw(|f| draw(f, &rows, &[], 0, None, &o, &mut dash))
            .unwrap();
        assert_ne!(dash.list_area, Rect::default());
    }

    // The rename header names the pinned session, not whatever row is selected.
    #[test]
    fn the_rename_header_names_the_pinned_session() {
        let rows = fixture();
        let r = Rename {
            key: rows[2].key(),
            buf: "new".into(),
        };
        let o = WatchOpts::default();
        let mut dash = test_dash();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| draw(f, &rows, &[], 0, Some(&r), &o, &mut dash))
            .unwrap();
        let buf = t.backend().buffer().clone();
        let head: String = (0..80u16).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(head.contains(&rows[2].label()), "{head:?}");
        assert!(!head.contains(&rows[0].label()), "{head:?}");
        assert!(head.contains("⏎ apply"), "{head:?}");
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        for (w, h) in [(1u16, 1u16), (10, 2), (20, 3), (28, 5), (200, 2)] {
            let s = screen(w, h, None);
            assert_eq!(s.len(), h as usize);
        }
    }

    // A terminal that advertises the kitty keyboard protocol gets exactly the
    // escape-code disambiguation push (flags = 1), and teardown pops precisely
    // what setup pushed — so Shift+Enter arrives as Enter+SHIFT (→ ESC CR for the
    // embedded program) without turning on release/repeat reporting.
    #[test]
    fn keyboard_enhancement_pushed_only_when_supported() {
        // Supported: push the DISAMBIGUATE flag and report that we did.
        let mut buf = Vec::new();
        let enhanced = push_keyboard_enhancement(&mut buf, true).unwrap();
        assert!(enhanced);
        assert_eq!(buf, b"\x1b[>1u");

        // Teardown pops exactly once for what was pushed.
        let mut buf = Vec::new();
        pop_keyboard_enhancement(&mut buf, enhanced).unwrap();
        assert_eq!(buf, b"\x1b[<1u");

        // Unsupported: write nothing and report nothing to pop, so the terminal's
        // flag stack is never underflowed on exit.
        let mut buf = Vec::new();
        let enhanced = push_keyboard_enhancement(&mut buf, false).unwrap();
        assert!(!enhanced);
        assert!(buf.is_empty());

        let mut buf = Vec::new();
        pop_keyboard_enhancement(&mut buf, enhanced).unwrap();
        assert!(buf.is_empty());
    }
}
