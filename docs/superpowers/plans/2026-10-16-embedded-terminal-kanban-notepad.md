# Embedded Terminal, PR Kanban and Notepad Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Do everything from the bearclaw dashboard: open tmux-backed Claude sessions inside it as a real terminal, see PRs as a tb-pr-style kanban, and keep plain-text notes in a Notepad tab edited in place.

**Architecture:** An `Editor` overlay (tui-textarea) replaces the `$EDITOR` suspend for every `Outcome::Edit`. A `Notes` tab (folder of `.md`/`.txt`, reusing `core::brain` listing) replaces Memos. The PRs view renders `Column`s as kanban lanes of cards. An `Embedded` terminal runs `tmux attach-session` in a PTY (portable-pty), parses it with `vt100`, draws the screen into the ratatui buffer and forwards keys encoded as xterm bytes; Sessions opens it on Enter for tmux sessions, `Ctrl+]` returns, `c` spawns a new tmux session with the configured launcher.

**Tech Stack:** Rust 2024, ratatui 0.29, crossterm 0.28, tui-textarea 0.7, vt100 0.15, portable-pty 0.9 (vt100 0.16 conflicts with ratatui 0.29's unicode-width pin).

**Spec:** this conversation, recorded here — user decisions: sessions live only in tmux and are used only through bearclaw, one session full-screen at a time, new sessions start like `cc yolo` (`claude --dangerously-skip-permissions`) through the user's shell, default directory `~/Desktop/projects/work`; PRs look like tb-pr's kanban; Notepad = own text files edited inside bearclaw, replacing Memos. Builder calls (user said "just do it"): leave key `Ctrl+]` (Esc belongs to Claude: interrupt/rewind), `c` = new session.

## Global Constraints

- Public repo: no personal paths/ids in code, tests, docs, commits (the launcher and default dir go in the user's config, not code).
- Tests never touch the user's real tmux sessions, terminals, config or home: throwaway tmux sessions are named `bearclaw-test-*` on a private socket (`tmux -L bearclaw-test-<pid>`) and killed; skip when tmux is missing.
- Nothing slow on the UI thread: PTY reads on a reader thread; spawn on a background thread.
- Gates per commit: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p bearclaw`.
- Update `docs/cli.md` for every key/config change.

## Review Focus

1. A non-tmux (iTerm) session → Enter still focuses its terminal; only tmux sessions embed.
2. The tmux session ends while embedded → the view says so and `Ctrl+]`/any key returns; no panic, no hung reader thread.
3. Resizing the outer terminal while embedded → the PTY and tmux client resize; no out-of-bounds cell writes.
4. Unsaved editor buffer + Esc → saved (autosave), never silently dropped; a file deleted under the editor is recreated on save.
5. PR kanban at 40 columns → one lane at a time with `‹ ›` paging; no panic at zero-size areas.

---

### Task 1: In-app editor overlay

**Files:** Create `crates/bearclaw/src/tui/tabs/editor.rs`; modify `tui/tabs/mod.rs`, `tui/watch.rs` (replace `suspend_edit` with the overlay), `crates/bearclaw/Cargo.toml` + workspace `Cargo.toml` (tui-textarea).

**Interfaces:** `pub struct Editor` — `open(path: PathBuf) -> std::io::Result<Editor>` (missing file = empty buffer), `draw(&mut self, f, area)`, `key(&mut self, k: KeyEvent) -> EditorOutcome { None, Closed(Result<(), String>) }`, `save(&mut self) -> Result<(), String>`, `dirty(&self) -> bool`. `Ctrl+S` saves; `Esc` saves if dirty then closes; `Ctrl+C` closes after saving too. In `watch.rs`, `Outcome::Edit(p)` sets `dash.editor = Some(Editor::open(p))`; while open it takes every key and draws over the body; on close the active tab gets `switch()` to re-read.

- [ ] Test `editor_saves_on_escape`: temp file "a\n"; open; type `bc`; `dirty()`; Esc → `Closed(Ok)`; file == "bca\n" (cursor starts at 0,0). `editor_creates_a_missing_file`: open a non-existent path, type `x`, Ctrl+S → file == "x". Run → fails (module missing). Implement. Pass. Docs: editor keys. Commit `feat(tui): in-app editor replaces $EDITOR`.

### Task 2: Notepad tab replaces Memos

**Files:** Modify `core/brain.rs` (`list_with(root, exts: &[&str])`, `list` = md only), create `core/notes.rs` (`root()` = config `notes.root` else `~/notes`), rename `tui/tabs/brain.rs` view into a shared `NotesView { kind }`-style list used by both Brain (md) and Notepad (md+txt) — or a second thin view reusing the same code; remove `tui/tabs/memos.rs` and `Tab::Memos`, add `Tab::Notes` ("Notepad", short "Not").

**Interfaces:** `brain::list_with(root: &Path, exts: &[&str]) -> Vec<Note>` (title for `.txt` = first non-empty line, else stem); `notes::root() -> PathBuf`; Notepad keys = Brain keys (`n` new — `.md` by default, `.txt` if the title ends in `.txt`; Enter/`e` open in the editor; `/` filter) plus `x` delete with a `y` confirm.

- [ ] Tests: `txt_notes_take_their_first_line_as_title`; `notepad_new_note_opens_the_editor` (`n`, title, Enter → `Outcome::Edit(path)`); `notepad_delete_needs_confirmation` (`x` then `n` keeps, `x` then `y` removes). Tab-bar test now lists "Notepad" instead of "Memos". Implement, docs (`notes.root`), commit `feat(tui): Notepad tab replaces Memos`.

### Task 3: PR kanban

**Files:** Modify `tui/tabs/prs.rs`.

**Interfaces:** `fn lanes(prs: &[Pr]) -> Vec<(Column, Vec<usize>)>` — non-empty columns in `Column::ALL` order; state `lane: usize`, `card: Vec<usize>` (selected card per lane), `first_lane: usize` (horizontal paging). Card (3–4 lines inside a rounded box): checks mark + title wrapped to 2 lines, `repo#N`, `size  age  💬N` (+ `draft`). Lane width ≥ 26 cols; lanes shown = `max(1, width / 26)`, paged to keep the selected lane visible, `‹`/`›` markers when more exist. Keys: ←/→ (and h/l) lane, ↑/↓ (j/k) card, Enter/`o` open, `r` refresh.

- [ ] Tests: `lanes_skip_empty_columns_in_order`; `kanban_moves_between_lanes_and_cards` (←/→ ↑/↓ change `selected_pr()`); `kanban_draws_cards_and_pages_on_narrow_screens` (130 cols: two lane titles with counts and a card's `api#1`; 40 cols: one lane and a `›` marker). Implement, docs, commit `feat(tui): PRs as a kanban`.

### Task 4: Key encoding for the embedded terminal

**Files:** Create `crates/bearclaw/src/tui/term/keys.rs` (`tui/term/mod.rs`).

**Interfaces:** `pub fn encode(k: KeyEvent) -> Option<Vec<u8>>` — chars (UTF-8), Enter `\r`, Shift/Alt+Enter `\x1b\r` (Claude's newline), Tab `\t`, BackTab `\x1b[Z`, Backspace `\x7f`, Esc `\x1b`, arrows `\x1b[A..D` (with modifiers `\x1b[1;{m}A`, m = 1+shift+2·alt+4·ctrl), Home/End `\x1b[H`/`\x1b[F`, PgUp/PgDn `\x1b[5~`/`\x1b[6~`, Delete `\x1b[3~`, F1–F4 `\x1bOP..S`, Ctrl+letter → 0x01–0x1a, Ctrl+Space 0x00, Alt+char → `\x1b` + char. `pub fn paste(text: &str) -> Vec<u8>` (bracketed `\x1b[200~…\x1b[201~`), `pub fn wheel(up: bool, col: u16, row: u16) -> Vec<u8>` (SGR `\x1b[<64;c;rM` / 65). `pub fn is_leave(k: KeyEvent) -> bool` — Ctrl+] (crossterm reports `Char(']')`+CONTROL or `Char('5')`+CONTROL).

- [ ] Test `keys_encode_like_xterm` covering every row above; `leave_is_ctrl_right_bracket`. Implement, commit `feat(tui): xterm key encoding`.

### Task 5: Embedded terminal widget

**Files:** Create `tui/term/embed.rs`; Cargo deps vt100 0.15, portable-pty 0.9.

**Interfaces:** `pub struct Embedded` — `attach(target: &str, rows: u16, cols: u16) -> Result<Embedded, String>` runs `tmux attach-session -t =<target>` (plus `BEARCLAW_TMUX_SOCKET`-style override for tests: `attach_cmd(cmd: CommandBuilder, rows, cols)`) in a PTY; a reader thread feeds bytes over mpsc; `pump(&mut self) -> bool` (drain into `vt100::Parser`, true when the screen changed), `resize(rows, cols)`, `write(bytes)`, `alive() -> bool`, `draw(&self, buf: &mut Buffer, area: Rect)` (cells → ratatui: fg/bg indexed+rgb, bold/italic/underline/inverse, wide chars, cursor cell reversed when visible). Before attaching, set the session's tmux options `status off`, `prefix None`, `mouse on`.

- [ ] Tests: `screen_cells_map_to_the_buffer` (feed a `vt100::Parser` `"\x1b[31mred\x1b[0m ok"` and draw → "red ok", first cell fg red); integration (skips without tmux) `embedded_tmux_round_trip`: private socket, `new-session -d -s bearclaw-test-embed cat`, attach via `attach_cmd`, `write(b"hello\r")`, pump until the screen contains `hello` (≤5 s), `resize(10, 40)`, kill server → `alive()` false within 5 s. Implement, commit `feat(tui): embedded tmux terminal`.

### Task 6: Sessions open inside bearclaw; new sessions in tmux

**Files:** Modify `tui/watch.rs`, `tui/dash/keys.rs` (`Action::NewSession` on `c`), `docs/cli.md`, `crates/bearclaw/SKILL.md`.

**Interfaces:** `Dash.term: Option<(String /*tmux session*/, Embedded)>`. Enter/tap/digit on a session with `backend == tmux` and a `tmux_session` → attach (error → event log, fall back to focus); other backends → `focus` as today. While `term` is open: every key except `is_leave` → `encode` → `write`; paste events → `paste`; wheel → `wheel`; the body below the tab bar is the terminal (resize on every draw to the body size); poll timeout 16 ms; when `!alive()` show "session ended — any key". `c` → `LineInput` "new session in" prefilled with the first `spawnDirs` entry for this host (else cwd) → background `backend::spawn_tagged(Backend::Tmux, dir, Prompt::None, None, None, false, launcher, Harness::Claude)` → on success attach to the returned tmux session. Help overlay + docs list Enter (embed), `Ctrl+]`, `c`.

- [ ] Tests: `c_opens_new_session` in keys.rs; watch golden `embedded_terminal_fills_the_body` using an `Embedded` built over a parser fed with text (no tmux) → row 0 tab bar, row 1.. the text. Implement; manual check in a throwaway tmux socket. Docs. Commit `feat(tui): sessions open inside bearclaw; c starts one in tmux`.

### Task 7: Local setup

- [ ] User config (not committed): `claude` = `claude --dangerously-skip-permissions`; `spawnDirs` first entry = `~/Desktop/projects/work` for this host; reinstall the binary; live smoke test in a throwaway tmux session.
