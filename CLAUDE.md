# bearclaw — contributor & agent guide

Supervise Claude Code sessions across machines: a Rust CLI (`bearclaw`) and a zero-dependency Node web
app. Read [README.md](README.md) and [docs/architecture.md](docs/architecture.md) first.

## Layout

```
Cargo.toml             workspace
crates/bearclaw/          the `bearclaw` binary + library
  src/core/            discovery, backends (tmux/iTerm), naming, tmux sessions, hosts/config — no printing
  src/cli/             argument handling and text/JSON rendering
  src/tui/             the `watch` dashboard (ratatui)
  SKILL.md             Claude Code skill installed by `bearclaw skill install`
  tests/               integration tests
crates/bearclaw-desktop/  Bearclaw.app: Tauri 2 shell around the web UI (not a default member — use -p bearclaw-desktop)
web/                   web server (server.mjs, lib/), Node >= 22, no npm deps
  ui/                  React + shadcn/ui PWA (npm, Vite build → ui/dist, gitignored) — see web/ui/README.md
docs/                  setup, architecture, CLI reference, migration
scripts/guide/         feature guide: demo API + headless-Chrome screenshots → one HTML page
```

Keep logic in `core`; the CLI, the TUI and any future UI (a second web UI, another TUI) are views
on it.

## Build & test

macOS: needs the Xcode Command Line Tools. If linking fails with an Xcode license error, prefix
cargo with `DEVELOPER_DIR=/Library/Developer/CommandLineTools` (or accept the license).

```sh
cargo build                       # from the repo root
cargo test                        # Rust unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo run -q -p bearclaw -- --help   # run the CLI from source
cargo test -p bearclaw-desktop       # the desktop app (crates/bearclaw-desktop/bundle.sh builds Bearclaw.app)

cd web && node --test tests/      # web tests (no network, child processes mocked)
cd web/ui && npm ci && npm run build && npm run lint && npm test   # React UI (bearclaw web build does ci+build)
cd web && node server.mjs         # run the web server against your local config
```

Tests use temp dirs and fixtures (`BEARCLAW_CONFIG`, `BEARCLAW_FIXTURE`, `HOME` pointing into a tempdir) —
never a real home directory, real config or live sessions.

## Git workflow

- This is a personal fork: `origin` is the fork, `upstream` the original project. Commit freely;
  **push only when the user asks**. Larger changes go on a branch.
- One machine, no deploy step: after a change, reinstall what changed —
  `cargo install --path crates/bearclaw` for the CLI/dashboard, `bearclaw web build` for the UI.
- Commit only your own files, because other sessions may have uncommitted work in the same tree.
  If another session is changing the same thing, move your work into a git worktree.
- Pulling upstream (`git fetch upstream && git merge upstream/main`): every `fleet` there is
  `bearclaw` here (binary, crates, `FLEET_*` env vars, paths), so expect conflicts on the name —
  resolve them to the bearclaw spelling.

## Rules

- **Privacy: this repo is public.** Never commit personal config, IP addresses (including
  Tailscale `100.x.y.z` addresses), hostnames, usernames, home paths (`/Users/<name>/…`), emails,
  ssh key names or tailnet names — not in code, tests, fixtures, docs or commit messages. Use
  placeholders: hosts `laptop` / `workstation`, `100.x.y.z`, `~/Code/project`, `$HOME`. Real values
  live only in the user's `~/.config/bearclaw/config.json`, which is never committed (`config.json` is
  gitignored).
- **JSON contracts are stable APIs**: `bearclaw list --json`, every other `--json` output, the config
  file shape and the web HTTP API. Adding fields is fine; renaming, removing or retyping one is a
  breaking change — bump the version and document it in `docs/`.
- **The config file is shared**: the CLI owns it, the web server only reads it. Preserve unknown
  keys on rewrite; a missing config must keep single-host local use working.
- **Commands that type into live agents** (`send`, `rename`, `name --apply`, `spawn`, `handoff`,
  web `send`/`keys`/`spawn`) must never resolve an ambiguous target — error with candidates.
- **Mutating tests against real tmux** use throwaway sessions named `bearclaw-test-*` and clean them
  up; never touch sessions you did not create.
- The web **server** (`web/server.mjs`, `web/lib/`) stays dependency-free (`node:`
  built-ins only) and build-free. npm dependencies live only in `web/ui/`, whose build output
  (`web/ui/dist`) is what gets served and installed.
- Update `docs/cli.md` when you change a command or flag, and `docs/architecture.md` when you
  change a contract.
