# Todo Tab and Session Sticky Notes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Todo tab of one-line work items that Claude sessions keep up to date themselves (and the user adds to), each optionally linked to its session and a Productive task, plus a per-session sticky note (≤ 6 bullets: what we're doing, what's done) shown on the Sessions tab.

**Architecture:** `core/todo.rs` keeps the list in one JSON file under the state dir, every change a locked read-modify-write; `core/sticky.rs` keeps one small file per session id. Sessions write through `bearclaw todo …` / `bearclaw sticky …`, which find the calling session with `discovery::origin()`; the SessionStart hook (`bearclaw brain hook`) tells every session to. The TUI gets a Todo tab and a sticky panel on Sessions.

**Tech Stack:** Rust, ratatui 0.29, serde_json, std file locks (`File::lock`).

**Spec:** user decisions in this conversation — Todo is its own tab (Notepad stays the user's); one line per item; an item may link a session and a Productive task (Enter opens the task in the browser); sessions add/update/finish their own item; the user adds items (`n`) and can start a session on one (`c`); sticky note per session, 5–6 bullets, written by the session as it goes, shown next to the selected session.

## Global Constraints

- Public repo: no personal data. Paths: `$BEARCLAW_TODO_FILE`, else `${XDG_STATE_HOME:-~/.local/state}/bearclaw/todo.json`; `$BEARCLAW_STICKY_DIR`, else `…/bearclaw/sticky/<session_id>.md`.
- Every todo write: lock `<file>.lock`, read, change, atomic write (tmp + rename), mode 0600.
- Item text and sticky bullets are one line each (newlines → spaces, trimmed); an item ≤ 120 chars, a bullet ≤ 160, ≤ 6 bullets.
- Tests never touch the real state dir or real sessions (env overrides, `--session` targets resolved from fixtures).
- Gates per commit: fmt, clippy `-D warnings`, `cargo test -p bearclaw`.

## Review Focus

1. Two sessions writing the todo at once → no lost item (lock).
2. `bearclaw todo set` from outside a Claude session → clear error naming `--session` / `todo add`, exit ≠ 0.
3. A session that ended → its doing item stays (marked with the ended session's label), sticky still readable by id.
4. Empty / missing files → empty list, no panic; a corrupt todo.json → error, file left untouched.
5. Narrow Sessions pane → the sticky panel moves below the list instead of squeezing it.

---

### Task 1: core/todo.rs
`Item { id: u64, text, status: Status{Open,Doing,Done}, session: Option<String>, session_label: Option<String>, task_url: Option<String>, created_ms, updated_ms }`; `Todo { version, next_id, items }`; `path()`, `load(path) -> Result<Todo>`, `update(path, f)`; ops on `Todo`: `add(text, task_url, session: Option<(id,label)>) -> u64` (Doing when a session is given), `set_for(session_id, label, text, task_url) -> u64` (the session's open/doing item is updated, else added), `claim(id, session) -> Result<()>`, `done(id) -> Result<()>`, `reopen(id)`, `remove(id)`, `doing_for(session_id) -> Option<&Item>`; `one_line(s, max) -> String`.
Tests: ops round trip; `set_for` updates instead of duplicating; `one_line` folds newlines and caps; concurrent `update` from 8 threads each adding one item → 8 items.

### Task 2: core/sticky.rs
`dir()`, `set(dir, session_id, bullets: &[String]) -> Result<Vec<String>>` (one-lined, empties dropped, ≤ 6), `get(dir, session_id) -> Option<Sticky { bullets, updated_ms }>`. Tests: caps and folds; missing → None.

### Task 3: CLI
`bearclaw todo [list] [--json]`, `todo add <text> [--task <url>]`, `todo set <text> [--task <url>] [--session <target>]`, `todo claim <id> [--session]`, `todo done [<id>] [--session]`, `todo rm <id>`; `bearclaw sticky set <bullet>... [--session]`, `sticky show [<target>] [--json]`. Calling session = `--session` target, else `discovery::origin()`; none → error "not inside a Claude session (pass --session <target>, or use `bearclaw todo add`)". `Placement::Here`. Integration tests with fixture sessions + env-overridden paths.

### Task 4: Hook context
`brain hook` output gains a "Your work: todo + sticky note" section (exact commands, one-line rule, ≤ 6 bullets, update after meaningful progress, `todo done` when finished, a `--task <url>` when a Productive task is known), and prints even when the brain folder is missing. Test: section present with and without a brain.

### Task 5: Todo tab
`Tab::Todo` ("Todo", "Tdo") after Tasks. Rows `● text  ⧉ session  ↗ task` (● doing, ○ open, ✓ done; done items from the last 3 days, dimmed, last). Keys: `n` add (input), Enter = open task link if any else go to the session, `g` go to session, `x` done/reopen, `d` delete (y/n), `c` start a Claude session on the item (prompt: "You're picking up todo item #<id>: <text>[ — <task url>]. Run `bearclaw todo claim <id>` first."), `r`. New `Outcome::Session(key)` and `Outcome::Spawn(prompt)` handled by the loop (switch to Sessions and open/spawn). Tests on the view with a temp todo file.

### Task 6: Sticky on Sessions
Selected session's todo line + sticky bullets in a panel: right of the list when the pane is ≥ 120 wide (width 44), else replacing the events pane (height ≤ 8). Embedded terminal header shows the session's todo line. Golden tests both widths.

### Task 7: Docs, SKILL.md, install, live check.
