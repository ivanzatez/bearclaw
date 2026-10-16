# Terminal Tabs and Brain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the `bearclaw` dashboard into a tabbed terminal app (Sessions, PRs, Tasks, Usage, Brain, Memos, Settings) and add a brain every Claude session reads and writes.

**Architecture:** `tui/watch.rs` keeps its loop and draws a tab bar on row 0; the Sessions tab is today's dashboard drawn below it, every other tab is a `View` in `tui/tabs/<name>.rs`. Slow sources load on background threads through a shared `Loader<T>`. New core modules: `core/brain.rs`, `core/productive.rs`, `core/webapi.rs`; new CLI: `bearclaw brain hook|install|path`.

**Tech Stack:** Rust 2024, ratatui 0.29, crossterm, serde_json (preserve_order), curl subprocesses.

**Spec:** `docs/superpowers/specs/2026-10-16-terminal-tabs-design.md`

## Global Constraints

- The repo is public: no org ids, tokens, hostnames, usernames, emails or `/Users/<name>` paths in code, tests, docs or commit messages. Placeholders only (`<org-id>`, `~/Code/project`).
- No new crate dependencies. HTTP via `curl` with secrets on stdin (`-K -`), as `core/usage.rs` does.
- No async runtime: `std::thread` + `std::sync::mpsc`.
- Keep logic in `core`; `tui` and `cli` are views on it.
- JSON contracts (`--json`, config shape, web API) unchanged; new config keys are additive.
- Update `docs/cli.md` for every new command, flag, key binding or config key.
- Gates before each commit: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

## Review Focus

1. Tasks tab with no token / no `productive.organizationId` / network down → one actionable line in the tab; other tabs keep working; the loop never blocks (test in Task 3).
2. Memos tab with the web server down → "web server not running … start it: bearclaw web serve" within ~3 s, no hang (test in Task 7).
3. Phone-width terminals (40 cols and less, 1-row areas) → tab bar falls back to short labels, then to `‹ Active ›`; no panic on zero-sized areas (test in Task 1).
4. Brain folder missing, empty, or holding a note with no heading → list still renders; `bearclaw brain hook` prints nothing and exits 0 on any error (tests in Tasks 5 and 6).
5. A text input is open (rename, search, send, new note title) → Tab, `q`, digits and Esc belong to the input, not to tab switching or quit (test in Task 1, re-asserted in Tasks 5 and 9).

---

### Task 1: Tab framework and tab bar

**Files:**
- Create: `crates/bearclaw/src/tui/tabs/mod.rs`, `tui/tabs/loader.rs`, `tui/tabs/pager.rs`, `tui/tabs/input.rs`
- Modify: `crates/bearclaw/src/tui/mod.rs` (add `pub mod tabs;`), `crates/bearclaw/src/tui/watch.rs` (loop + draw), `docs/cli.md` (watch keys)

**Interfaces:**
- Produces (`tui/tabs/mod.rs`):
  - `enum Tab { Sessions, Prs, Tasks, Usage, Brain, Memos, Settings }` with `ALL: [Tab; 7]`, `title() -> &'static str` ("Sessions", "PRs", "Tasks", "Usage", "Brain", "Memos", "Settings"), `short() -> &'static str` ("Ses", "PR", "Tsk", "Use", "Brn", "Mem", "Set"), `next()`, `prev()` (wrapping).
  - `fn tab_bar(active: Tab, width: u16, badge: Option<String>) -> (Line<'static>, Vec<(Tab, u16, u16)>)` — the line plus each tab's `[x0, x1)` hit range; `badge` is appended to the Sessions label (e.g. `⏸2`).
  - `fn hit(ranges: &[(Tab, u16, u16)], x: u16) -> Option<Tab>`.
  - `enum Outcome { None, Quit, Home, Edit(PathBuf), Log(String) }` — `Home` = go to Sessions, `Edit` = suspend the TUI and open the path in `$EDITOR`, `Log` = one line for the events log.
  - `trait View { fn on_show(&mut self); fn tick(&mut self); fn draw(&mut self, f: &mut Frame, area: Rect); fn key(&mut self, k: KeyEvent) -> Outcome; fn typing(&self) -> bool; }`
  - `struct Tabs { pub active: Tab, views: Vec<Box<dyn View>> /* one per non-Sessions tab, ALL order */ }` with `new() -> Tabs`, `view(&mut self, t: Tab) -> Option<&mut dyn View>` (None for Sessions), `switch(&mut self, t: Tab)` (calls `on_show`), `tick_all(&mut self)`.
  - `struct Placeholder(&'static str)` implementing `View` — later tasks replace it tab by tab.
- Produces (`loader.rs`): `struct Loader<T> { pub data: Option<T>, pub error: Option<String>, pub loading: bool, ... }` with `new()`, `start(&mut self, f: impl FnOnce() -> Result<T, String> + Send + 'static)` (no-op while loading), `poll(&mut self) -> bool` (true when something arrived), `stale(&self, ttl: Duration) -> bool` (never fetched, or older than ttl). An error keeps the previous `data`.
- Produces (`pager.rs`): `struct Pager` with `new(title: String, text: String)`, `draw(&mut self, f, area)`, `key(&mut self, k) -> bool` (false = close: Esc, `q`, Left, Backspace). j/k/↑/↓ scroll by one, PgUp/PgDn/space by a page, g/G top/bottom; text wraps.
- Produces (`input.rs`): `struct LineInput { pub prompt: String, pub buf: String }` with `key(&mut self, k) -> InputResult { Editing, Submit(String), Cancel, Quit }` built on `dash::keys::rename_action`, and `draw(&self, f, area)` (`prompt: buf▏`).

- [ ] **Step 1: Tests** in `tabs/mod.rs`, `loader.rs`, `pager.rs`, `input.rs` (`#[cfg(test)]`):
  - `tab_cycle_wraps`: `Tab::Settings.next() == Tab::Sessions`, `Tab::Sessions.prev() == Tab::Settings`.
  - `tab_bar_degrades_with_width`: at 120 cols the line contains every `title()`; at 40 cols every `short()`; at 14 cols it is `‹ Sessions ›` style (active only) and `width_of(line) <= 14`; at 0 cols no panic. `hit` on the middle of the "PRs" range returns `Some(Tab::Prs)`.
  - `loader_delivers_and_keeps_data_on_error`: `start(|| Ok(5))` then poll (≤2 s) → `data == Some(5)`, `!loading`; `start(|| Err("x".into()))` → `data == Some(5)`, `error == Some("x")`; `stale(Duration::from_secs(60))` is false right after a fetch, true for a fresh loader.
  - `pager_scrolls_and_closes`: 100-line text in a 10-row area: `G` then draw shows line 100; Esc returns false.
  - `line_input_owns_command_keys`: typing `q`, `1`, Tab-less chars append to `buf`; Enter → `Submit(buf)`; Esc → `Cancel`; Ctrl-C → `Quit`.
- [ ] **Step 2:** `cargo test -p bearclaw tabs` → fails (module missing).
- [ ] **Step 3: Implement** the four files.
- [ ] **Step 4: Wire into `watch.rs`:**
  - Split `draw` into `draw_in(f, area, rows, log, sel, renaming, o, dash)` and keep `draw(f, …)` = `draw_in(f, f.area(), …)` so the existing golden tests stay valid; `render_help` stays full-frame.
  - New `draw_app(f, tabs, rows, log, sel, renaming, o, dash)`: row 0 = `tab_bar` (Sessions badge = number of waiting sessions when > 0), store hit ranges in `Dash.tab_hits`; below it either `draw_in` (Sessions) or the active view's `draw`.
  - Loop: `tabs.tick_all()` each iteration; Tab / BackTab switch tabs unless the rename buffer is open or the active view is `typing()`; a left click on row 0 switches via `hit`; on a non-Sessions tab every other key goes to the view and its `Outcome` is applied (`Quit` breaks, `Home` → Sessions, `Log` → events log, `Edit(p)` → `suspend_edit(&mut terminal, mouse, &p)` which leaves raw mode/alt screen/mouse capture, calls `cli::config_cmd::open_editor`, restores them and clears the terminal). Session polling and notifications keep running on every tab.
  - Help overlay lists `Tab/⇧Tab  switch tab`.
- [ ] **Step 5: Golden test** `golden_tab_bar_on_top` in `watch.rs`: render `draw_app` at 120×30 with the fixture → row 0 contains "Sessions" and "Settings", row 1 contains "3 live"; with active = Prs, row 1 contains the placeholder text.
- [ ] **Step 6:** `cargo test -p bearclaw` → all pass (existing golden tests unchanged).
- [ ] **Step 7:** Update `docs/cli.md` watch section (tabs, Tab/⇧Tab, click). Commit `feat(tui): tab bar and tab framework`.

### Task 2: PRs tab

**Files:**
- Create: `crates/bearclaw/src/tui/tabs/prs.rs`
- Modify: `crates/bearclaw/src/core/tools.rs` (`open_url`), `tui/tabs/mod.rs` (register)

**Interfaces:**
- Consumes: `Loader<T>`, `View`, `Outcome`; `core::prs::{Settings::load, get, Mode, View as PrView, Pr, Column}`.
- Produces: `pub fn open_url(url: &str) -> Result<(), String>` in `core/tools.rs` (`open` on macOS, else `xdg-open`; refuses non-`http(s)://` urls); `pub struct PrsView` implementing `View`.

- [ ] **Step 1: Tests:** `open_url_rejects_non_http` (`"file:///etc/passwd"` and `"javascript:x"` → Err). In `prs.rs`: `rows_group_by_column` — build a `Snapshot` with PRs in `WaitingOnMe` and `DraftMine`; the flattened row list puts a `Column::title()` header before each group in `Column::ALL` order and selection skips headers; `draw` at 100×20 shows `owner/repo#N` and the title; with `error = Some("gh: not logged in")` and no data the area shows that error.
- [ ] **Step 2:** run → fails.
- [ ] **Step 3: Implement.** `on_show` starts `prs::get(&Settings::load(), Mode::Auto, now)` when stale (ttl 5 min); `r` = `Mode::Refresh`; Enter/`o` = `open_url(pr.url)` (error → `Outcome::Log`); j/k/↑/↓ move over PR rows only; Esc → `Home`, `q` → `Quit`. Row: `repo#N  title  @author  size  checks  age`; a `stale` view shows `(cached — <error>)` in the block title.
- [ ] **Step 4:** tests pass. **Step 5:** docs (`docs/cli.md` PRs tab keys); commit `feat(tui): PRs tab`.

### Task 3: Productive tasks in core + Tasks tab

**Files:**
- Create: `crates/bearclaw/src/core/productive.rs`, `crates/bearclaw/src/tui/tabs/tasks.rs`
- Modify: `core/mod.rs`, `tui/tabs/mod.rs`, `docs/cli.md` (config keys)

**Interfaces:**
- Produces (`core/productive.rs`):
  - `struct Settings { pub org_id: String, pub token_env: String, pub api_url: String, pub app_url: String }`; `fn Settings::from_raw(raw: &Value) -> Result<Settings, String>` reads `productive.organizationId` (required; error text: `set productive.organizationId: bearclaw config set productive.organizationId <id>`), `tokenEnv` (default `PRODUCTIVE_AUTH_TOKEN`), `apiUrl` (default `https://api.productive.io/api/v2`), `appUrl` (default `https://app.productive.io`); `Settings::load()` = `from_raw(&config::load_from(&config::path()).raw)`.
  - `struct Task { pub id: String, pub number: String, pub title: String, pub project: String, pub status: String, pub due_date: Option<String>, pub updated_at: String, pub url: String, pub description: String }`.
  - `fn parse_membership(v: &Value, org_id: &str) -> Option<(String, String)>` — `(person_id, org_name)` from `/organization_memberships?include=organization`.
  - `fn parse_tasks(v: &Value, task_url_base: &str) -> Vec<Task>` — joins `included` projects (`name`) and workflow_statuses (`name`); `url` = `{task_url_base}/tasks/{id}`; `description` = `html_to_text`.
  - `fn html_to_text(html: &str) -> String` — `<br>`, `</p>`, `</li>` → newline, `<li>` → `- `, strip other tags, decode `&amp; &lt; &gt; &quot; &#39; &nbsp;`.
  - `core::tools::slugify(name: &str) -> String` (new, shared with Task 5) — lowercase, runs of non-alphanumerics → one `-`, trimmed (`"Acme Ltd."` → `"acme-ltd"`); task url base = `{app_url}/{org_id}-{slugify(org_name)}`.
  - `fn sort_tasks(tasks: &mut [Task])` — due date ascending (none last), then `updated_at` descending.
  - `fn my_open_tasks(s: &Settings) -> Result<Vec<Task>, String>` — token from env (missing → `no Productive token: set $PRODUCTIVE_AUTH_TOKEN`), `curl -sSg --max-time 15 -K -` with `X-Auth-Token` / `X-Organization-Id` headers on stdin, `-w "\n%{http_code}"`; non-2xx → `Productive API <code>: <first error detail>`. Calls: memberships, then `/tasks?filter[assignee_id]=<person>&filter[status]=1&include=project,workflow_status&page[size]=200`.
- Produces (`tui/tabs/tasks.rs`): `pub struct TasksView` implementing `View`.

- [ ] **Step 1: Tests** (fixture JSON inline, ids made up):
  - `settings_need_an_org`: `{}` → Err containing `productive.organizationId`; `{"productive":{"organizationId":"42"}}` → defaults filled.
  - `membership_picks_the_configured_org`: two memberships (orgs "1" and "42") with included orgs → `parse_membership(v, "42") == Some(("p42", "Acme Ltd."))`.
  - `tasks_join_project_and_status`: one task with project and workflow_status relationships → `project == "Website"`, `status == "In progress"`, `url == "https://app.example/42-acme-ltd/tasks/7"`, `description == "Fix it\n- one\n- two"` from `<p>Fix it</p><ul><li>one</li><li>two</li></ul>`.
  - `overdue_first_then_recent`: due `2026-01-02`, none, `2026-01-01` → order 01-01, 01-02, none.
  - In `tasks.rs`: `tasks_tab_shows_errors_inline` — a view whose loader holds `error = Some("set productive.organizationId …")` draws that text at 80×10 and `key(Tab-less 'j')` does not panic with no data.
- [ ] **Step 2:** run → fails. **Step 3: Implement.** Tab: ttl 2 min; rows `#number  title  project  status  due` (overdue due dates red); Enter opens a `Pager` with title, project, status, due, url and description; `o` = `open_url`; `r` refresh; Esc → `Home` (or closes the pager); `q` → `Quit`.
- [ ] **Step 4:** tests pass. **Step 5:** docs (config keys `productive.*`, Tasks keys); commit `feat: Productive tasks tab`.

### Task 4: Usage tab

**Files:** Create `crates/bearclaw/src/tui/tabs/usage.rs`; modify `tui/tabs/mod.rs`.

**Interfaces:** Consumes `core::usage::{get, Usage, Limit}` and `discovery::claude_home()`. Produces `pub struct UsageView` implementing `View`.

- [ ] **Step 1: Test** `usage_draws_a_gauge_per_limit`: a `Usage` with limits "Current session" 42% and "Weekly · all models" 87% (severity "warn") drawn at 80×12 → lines contain both labels, "42%" and "87%", and the reset time; `stale = true` shows `stale` in the title.
- [ ] **Step 2:** fails. **Step 3: Implement** — ttl 60 s (the core cache already dedupes), `r` = `get(home, true)`; one `LineGauge` per limit grouped "Session" / "Weekly", account plan line on top, extra usage line when enabled; red ≥ 90%, yellow ≥ 70%.
- [ ] **Step 4:** pass. **Step 5:** commit `feat(tui): usage tab`.

### Task 5: Brain in core + Brain tab

**Files:**
- Create: `crates/bearclaw/src/core/brain.rs`, `crates/bearclaw/src/tui/tabs/brain.rs`
- Modify: `core/mod.rs`, `tui/tabs/mod.rs`, `docs/cli.md`

**Interfaces:**
- Produces (`core/brain.rs`):
  - `fn root() -> PathBuf` — config `brain.root` (`~` expanded, read via `config::load_from(&config::path())`), else `~/brain`.
  - `struct Note { pub path: PathBuf, pub rel: String, pub title: String, pub mtime_ms: i64 }`.
  - `fn list(root: &Path) -> Vec<Note>` — recursive `*.md`, skipping dot-dirs and `node_modules`, newest first; a missing root → empty.
  - `fn title_of(text: &str, stem: &str) -> String` — first `# ` heading after optional `---` frontmatter, else `stem`.
  - `fn search(notes: &[Note], q: &str) -> Vec<usize>` — every whitespace-separated term (case-insensitive) appears in the title or the file text; empty `q` → all.
  - `fn create(root: &Path, title: &str) -> std::io::Result<PathBuf>` — creates root if missing, writes `# {title}\n\n` to `{slugify(title)}.md` (empty slug → `note`), `-2`, `-3`… on collision, never overwrites.
- Produces (`tui/tabs/brain.rs`): `pub struct BrainView` implementing `View`.

- [ ] **Step 1: Tests** (tempdir roots):
  - `titles_come_from_the_first_heading`: `"---\ntags: x\n---\n# Deploys\nbody"` → "Deploys"; `"no heading"` → stem.
  - `list_skips_hidden_and_survives_a_missing_root`: files `a.md`, `.git/x.md`, `sub/b.md`, `c.txt` → rels `["sub/b.md", "a.md"]` order by mtime; `list(missing)` is empty.
  - `search_needs_every_term`: notes "Deploy api" (body "uses helm") and "Deploy web" → `search("deploy helm")` = only the first.
  - `create_never_overwrites`: creating "Deploys" twice → `deploys.md`, `deploys-2.md`, first file unchanged.
  - In `brain.rs`: `brain_tab_filters_while_typing` — `/` opens search (`typing() == true`), typing `helm` narrows the list, `q` is typed into the query (no `Quit`), Esc closes search; `n` + title + Enter returns `Outcome::Edit(path)` of the new file.
- [ ] **Step 2:** fails. **Step 3: Implement.** Tab: list `title  rel  age`, re-listed on `on_show` and after an edit; `/` live filter (`LineInput`), Enter reads (`Pager`), `e` → `Outcome::Edit(selected)`, `n` → title input → `create` → `Outcome::Edit(new)`, `r` re-list, Esc → `Home`, `q` → `Quit`. Empty brain shows `no notes yet in <root> — n to write one`.
- [ ] **Step 4:** pass. **Step 5:** docs (`brain.root`, Brain keys); commit `feat: brain notes and Brain tab`.

### Task 6: `bearclaw brain hook|install|path`

**Files:**
- Create: `crates/bearclaw/src/cli/brain.rs`
- Modify: `crates/bearclaw/src/core/brain.rs` (hook text), `cli/mod.rs`, `main.rs` (subcommand), `crates/bearclaw/SKILL.md` (one paragraph), `docs/cli.md`

**Interfaces:**
- Consumes: `core::brain::{root, list}`, `discovery::claude_home()`.
- Produces:
  - `core::brain::hook_context(root: &Path, notes: &[Note], cap: usize) -> String` — the text below plus up to `cap` lines `- {title} — {rel}`, then `(+N more — search the folder)`; total ≤ 8000 chars.
  - `core::brain::install_hook(settings: &mut Value, command: &str) -> bool` / `uninstall_hook(settings: &mut Value) -> bool` — pure edits of a parsed `settings.json`; ours = any `hooks.SessionStart[].hooks[]` entry whose `command` ends with `brain hook`; install replaces ours in place or appends `{"hooks":[{"type":"command","command":<command>}]}`; returns whether anything changed; every other key and hook untouched.
  - `cli::brain::BrainAction { Hook, Install { uninstall: bool }, Path }`.
- Hook context text (exact copy):

```
# Brain — shared notes for every Claude session on this machine
The brain is a folder of markdown notes at {root}. Every Claude Code session here reads and writes it.
- Read the notes below whose titles look relevant before you start (Read tool); Grep the folder for topics not listed.
- When you learn something durable and reusable — how a system works, a gotcha, a decision and its reason, a preference of the user — write it down: update the note that covers the topic, or create {root}/<topic-slug>.md starting with "# Title". One topic per note. Task progress, temporary state and secrets do not belong here.
- When the user asks you to remember something, save it to the brain, or to check the brain, this is the place.
Notes (newest first):
```

- [ ] **Step 1: Tests:**
  - `hook_context_lists_and_caps`: 3 notes, cap 2 → contains the root, the first two titles, `(+1 more`; length ≤ 8000.
  - `install_is_idempotent_and_preserving`: settings `{"model":"x","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"other"}]}],"Stop":[]}}` → install appends ours (2 entries, "other" kept, `model` and `Stop` kept, key order kept); install again with a new path → still 2 entries, command updated; `uninstall_hook` → back to the original value.
  - Integration test in `crates/bearclaw/tests/cli.rs`: `brain_hook_never_fails` — `HOME`/`BEARCLAW_CONFIG` in a tempdir with `brain.root` pointing at a missing dir → `bearclaw brain hook` exits 0 with empty stdout; with one note → stdout parses as JSON with `hookSpecificOutput.hookEventName == "SessionStart"` and `additionalContext` containing the note title.
- [ ] **Step 2:** fails. **Step 3: Implement.** `hook` prints `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":…}}` (cap 100) and swallows every error (exit 0, no output). `install` creates the root, reads `<claude_home>/settings.json` (missing → `{}`, unparseable → error, file untouched), applies `install_hook` with `"<current_exe> brain hook"`, writes atomically (tmp + rename), prints what changed; `--uninstall` reverses. `path` prints the root.
- [ ] **Step 4:** pass. **Step 5:** docs + SKILL.md paragraph; commit `feat: bearclaw brain hook/install/path`.

### Task 7: Local web API client + Memos tab

**Files:**
- Create: `crates/bearclaw/src/core/webapi.rs`, `crates/bearclaw/src/tui/tabs/memos.rs`
- Modify: `core/mod.rs`, `cli/brief.rs` (move `local_web_base` to `core::webapi::base_url`, keep a re-export or update callers), `tui/tabs/mod.rs`

**Interfaces:**
- Produces: `core::webapi::base_url() -> String`; `core::webapi::get_json(path: &str) -> Result<Value, String>` — `curl -sS --max-time 3 -w "\n%{http_code}" <base><path>`; connection failure (curl exit 7/28) → `web server not running at <base> — start it: bearclaw web serve (or bearclaw web install-service)`; non-2xx → the body's `error` field; `pub fn split_status(out: &str) -> (String, u16)`.
- `struct Memo { id, title, created: Option<String>, duration_sec: Option<f64>, status, words: u64, preview }` + `parse_memos(v: &Value) -> Vec<Memo>` in `memos.rs`; `pub struct MemosView` implementing `View`.

- [ ] **Step 1: Tests:** `split_status` on `"{\"a\":1}\n200"` → (`{"a":1}`, 200) and on `"\n000"` → ("", 0); `get_json_reports_a_dead_server` — `BEARCLAW_CONFIG` pointing at a config with `web.port` of a closed local port → Err contains `bearclaw web serve`, returns in < 4 s; `parse_memos` on a two-memo body keeps order and fields; `MemosView` with that error draws it.
- [ ] **Step 2:** fails. **Step 3: Implement.** Tab: ttl 60 s; rows `title  created  duration  words  status`; Enter loads `/api/memos/<id>` in a loader and opens a `Pager` with the transcript; `r`, Esc → `Home`, `q` → `Quit`.
- [ ] **Step 4:** pass. **Step 5:** commit `feat(tui): memos tab`.

### Task 8: Settings tab

**Files:** Create `crates/bearclaw/src/tui/tabs/settings.rs`; modify `tui/tabs/mod.rs`.

**Interfaces:** Consumes `config::{path, load_from}`, `core::brain::root`, `productive::Settings::from_raw`, `repos::Settings` / raw `repos`, `webapi::base_url`. Produces `pub struct SettingsView` implementing `View`.

- [ ] **Step 1: Test** `settings_summarise_the_config`: a temp config with `spawnDirs` (one label), `brain.root`, `productive.organizationId` → draw at 100×30 shows the config path, the spawn dir label and path, the brain root and "hook installed: no", "Productive org: <id> · token: missing" (env var unset in the test via a custom `tokenEnv`).
- [ ] **Step 2:** fails. **Step 3: Implement.** Sections: Config file · Host (self, default host) · Web (dir, port, bind, server reachable — checked in a loader) · Brain (root, notes count, hook installed?) · Productive (org, token present?) · Spawn dirs · Repos (roots, every) · PRs (orgs/repos). `e` → `Outcome::Edit(config::path())`, then re-read on the next `on_show`/`r`. Esc → `Home`, `q` → `Quit`.
- [ ] **Step 4:** pass. **Step 5:** commit `feat(tui): settings tab`.

### Task 9: Session detail view

**Files:** Create `crates/bearclaw/src/tui/tabs/session.rs`; modify `tui/watch.rs`, `tui/dash/keys.rs` (`Action::Detail`), `docs/cli.md`.

**Interfaces:**
- Consumes: `backend::{peek, send, focus}`, `core::brief::{briefs_dir, load}`, `Loader`, `LineInput`.
- Produces: `Action::Detail` mapped from `d` and Right; `pub struct SessionDetail { pub key: String, … }` with `open(key: String) -> SessionDetail`, `tick(&mut self, rows: &[Session])`, `draw(&mut self, f, area, rows: &[Session])`, `key(&mut self, k, rows: &[Session]) -> DetailOutcome { None, Close, Quit, Log(String) }`, `typing(&self) -> bool`. Stored in `Dash.detail: Option<SessionDetail>`.

- [ ] **Step 1: Tests:** in `keys.rs` `d_and_right_open_detail`; in `session.rs` `detail_pins_its_session_and_sends_through_input`: with fixture rows, `open(key)` then `draw` at 100×30 shows the headline and cwd; the session missing from rows draws `session ended`; `s` opens the input (`typing()`), typing `q` stays in the buffer, Esc cancels; a `peek` error (fixture mode refuses peeks) is shown inline instead of panicking.
- [ ] **Step 2:** fails. **Step 3: Implement.** Layout: header (headline · status · cwd · host), brief summary + open todos when the brief exists, then the peek tail filling the rest (peek in a `Loader<String>`, refreshed every 2 s while open, via `backend::peek` on a cloned `Session`), input line at the bottom when sending. Keys: `s` send (Enter sends in a background thread, result → `Log`), Enter/`o` focus, `r` re-peek, Esc/Left close. In `watch.rs`: `Action::Detail` opens it for `rows[sel]`; while open, keys route to it, `draw_in` draws it instead of the list (header row stays), the help overlay mentions `d`.
- [ ] **Step 4:** `cargo test -p bearclaw` all pass. **Step 5:** docs; commit `feat(tui): session detail view`.

### Task 10: Repo housekeeping for the fork

**Files:** Modify `CLAUDE.md` (git workflow), `README.md` (dashboard section + brain).

- [ ] **Step 1:** `CLAUDE.md` git workflow: single machine, commit to `main`, push only when asked, no multi-host deploy step; keep the privacy rules. README: a short "Dashboard tabs" and "Brain" section pointing at `docs/cli.md`.
- [ ] **Step 2:** full gates: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`, `cd web && node --test tests/`. Commit `docs: fork workflow, tabs and brain`.
