# Terminal tabs and the brain — design

## Why

bearclaw should be usable entirely from one terminal. Today the dashboard (`bearclaw` / `bearclaw
watch`) only lists sessions; everything else (PRs, usage, notes, memos, settings, session detail)
lives in the web UI. The user also wants a **brain**: notes every Claude Code session on the
machine reads and writes, on its own and when asked.

## Goals

- One dashboard with tabs: **Sessions · PRs · Tasks · Usage · Brain · Memos · Settings**.
- Switch with **Tab / Shift-Tab** or a click on the tab bar (digits stay "jump to session").
- Sessions keeps everything it does today, plus a **detail view** of one session: what its
  screen shows (peek), its brief (summary + todos), and sending it a message.
- **Tasks**: the user's open Productive tasks (any Productive org, configured by id).
- **Brain**: a folder of markdown notes. Browse, search, read, create, edit (in `$EDITOR`).
- Every Claude Code session gets the brain's index at start (a SessionStart hook installed by
  `bearclaw brain install`) with instructions to read relevant notes and write durable knowledge.
- No tab ever freezes the UI: slow fetches (gh, curl) run on background threads.

## Non-goals

- Jobs and Operator tabs (not wanted; the web UI keeps them).
- Recording memos from the terminal (read-only Memos tab).
- Editing settings in-place (Settings shows them; `e` opens the config in `$EDITOR`).
- Changing the web UI.

## Anti-goals

- A slow or failing source (no token, no web server, GitHub down) blocking or breaking other tabs.
- A broken brain (missing folder, odd files) breaking Claude sessions — the hook prints nothing
  and exits 0 on any error.
- Personal data in the repo: org ids, tokens, hostnames and paths live only in the user's config.

## Decisions

- Tabs are built on the existing ratatui loop in `tui/watch.rs`; each tab is its own module under
  `tui/tabs/`. Background work uses `std::thread` + `mpsc`, like the naming pool (no async).
- HTTP goes through `curl` (the crate has no HTTP client, by design), with secrets passed on stdin
  (`-K -`) like `core/usage.rs`.
- Memos read from the local web server (`/api/memos`); when it is not running the tab says how to
  start it. The Brain tab reads files directly, so it needs no server.
- Productive: config `productive.organizationId` (required), token from env
  `PRODUCTIVE_AUTH_TOKEN` (name configurable via `productive.tokenEnv`). The person id and org name
  are discovered from `/organization_memberships`.
- Brain root: config `brain.root`, default `~/brain`. Notes are `*.md`; the title is the first
  `# ` heading, else the file name.

## Left to the builder

Exact column layouts, colours, footer hint wording, and how each list degrades at phone widths,
following the existing dashboard's style (`tui/dash/layout.rs`).
