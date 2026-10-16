# bearclaw

Supervise a bearclaw of Claude Code sessions across your machines — from a terminal or from your phone.

`bearclaw` is one Rust CLI plus a small web app (a zero-dependency Node server and a React PWA). It finds every live Claude Code
session on a machine (in tmux panes or iTerm tabs), shows what each one is doing, and lets you
peek at, steer, rename, spawn and hand work off to them. [Codex](https://github.com/openai/codex)
and [pi](https://pi.dev) sessions are found and driven too — pick the agent and its model when you
spawn (`bearclaw spawn --harness codex`, or the web New session form); see
[docs/cli.md → Harnesses](docs/cli.md#harnesses). It also manages the tmux sessions on an
always-on **workstation** over ssh, so the same command works from your laptop and on the
workstation itself.

The setup it is built for:

- an always-on Mac (the *workstation*) runs Claude Code inside tmux, 24/7;
- your laptop and phone reach it over [Tailscale](https://tailscale.com);
- closing the laptop lid never interrupts an agent — the tmux server lives on the workstation.

## Architecture

```
  laptop                         phone                           workstation (always on)
 ┌──────────────────────┐      ┌──────────────────┐            ┌──────────────────────────────┐
 │ bearclaw <cmd>          │      │ browser / PWA    │            │ tmux server (24/7)           │
 │  ├─ local sessions   │      │  (bearclaw web UI)  │            │  ├─ job-a: claude            │
 │  └─ -H workstation ──┼─ssh──┼──────────────────┼──────────▶ │  ├─ job-b: claude            │
 │                      │      │                  │   http     │  └─ bearclaw: bearclaw watch (TUI) │
 │ bearclaw web serve :7777│◀─────┼── http ──────────┼──────────▶ │ bearclaw web serve :7777        │
 └──────────────────────┘ peer │                  │  (tailnet) └──────────────────────────────┘
            ▲                  └──────────────────┘                        ▲
            └─────────────── Tailscale (WireGuard mesh, private IPs) ──────┘
```

- **CLI** — `bearclaw` discovers sessions from Claude Code's own registry (`~/.claude/sessions/`),
  drives them through tmux or iTerm, and dispatches to other hosts by re-running itself over ssh
  (`ssh <host> bearclaw <same args>`).
- **Web** — every machine runs the same server. It serves its own sessions and merges in its
  peers', so any one URL shows the whole bearclaw. Built for a phone: list, chat/terminal view,
  send a message, spawn a session.
- **Config** — one small JSON file per machine (`~/.config/bearclaw/config.json`), written by
  `bearclaw init`, read by both the CLI and the web server.

Details: [docs/architecture.md](docs/architecture.md).

## Quickstart

Requirements: macOS (the only tested platform; iTerm support is macOS-only), tmux, Claude Code, Rust (to build),
Node ≥ 22 (for the web app), Tailscale + ssh between machines for multi-host use.

Build prerequisites: a Rust toolchain ([rustup](https://rustup.rs); the repo pins it in
`rust-toolchain.toml`) and, on macOS, the Xcode Command Line Tools (`xcode-select --install`) for
the linker. If a full Xcode is installed but its license was never accepted, linking fails with
"You have not agreed to the Xcode license agreements" — run `sudo xcodebuild -license accept`, or
build against the Command Line Tools instead:
`DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo install --path crates/bearclaw`.

```sh
git clone https://github.com/ivanzatez/bearclaw && cd bearclaw
cargo install --path crates/bearclaw     # installs `bearclaw` into ~/.cargo/bin
bearclaw web build                       # build the web UI (npm ci + vite build → web/ui/dist)

bearclaw init                            # interactive: this machine's name, hosts, spawn dirs, web port
bearclaw install --host workstation      # copy the binary + web app (incl. built UI) to the workstation
bearclaw doctor                          # reachability, versions on both ends, config sanity

bearclaw list                            # Claude sessions on this machine
bearclaw -H workstation list             # ...on the workstation
bearclaw web serve                       # web UI on :7777 (run it on each machine)
```

Then open `http://<workstation-tailscale-ip>:7777` on your phone and add it to the home screen. On a
Mac, `crates/bearclaw-desktop/bundle.sh --install` builds **Bearclaw.app**, the same UI as a desktop app
([docs/desktop.md](docs/desktop.md)).

Building the whole setup from scratch (always-on Mac, Tailscale, ssh, tmux, phone):
[docs/setup.md](docs/setup.md).

## Commands

| area | commands |
| --- | --- |
| Claude sessions | `list` · `peek` · `send` · `rename` · `name` · `spawn` · `handoff` · `watch` (default) · `skill` |
| tmux sessions | `tmux list\|enter\|last\|new\|kill\|rename\|stale` (alias `t`), shortcuts `enter` · `last` · `new` |
| after a reboot | `restore` — dormant sessions: list, bring back (same tmux layout, `claude --resume`), forget |
| hosts | `exec` · `ssh` · `doctor` |
| setup | `init` · `config path\|show\|edit\|set` · `install --host <name>` |
| pull requests | `prs` — the GitHub PRs that need you (unseen @mentions, yours, waiting on your review, waiting on their author), through `gh`, with their local worktrees |
| git repos | `repos` · `repos sync` · `repos install-service` — fetch + fast-forward every repo under `~/Code`, on a timer |
| scheduled jobs | `jobs` · `jobs show\|run\|enable\|disable` · `jobs install-service` — cron-scheduled agent sessions on a host, like Claude Code routines; everything a job session spawns joins its run |
| web | `web serve` · `web build` · `web install-service` |
| agents | `mcp` — every command above as an MCP tool, for Claude Code and other agents |

Global flags: `-H, --host <name>`, `--local`, `--json` (where it applies). Full reference:
[docs/cli.md](docs/cli.md).

A few examples:

```sh
bearclaw                                   # live dashboard (TUI); piped, it prints `list`
bearclaw peek docs-refresh                 # what that session shows right now
bearclaw send docs-refresh "run the tests" # type into it and press Enter
bearclaw list -a --json                    # every host, one array, rows tagged with host
bearclaw new billing-fix -C ~/Code/project # attach to (or create) a tmux session on the workstation
bearclaw last                              # back to the previous tmux session
bearclaw tmux stale                        # idle shells nothing is using
```

## Docs

- [docs/setup.md](docs/setup.md) — end-to-end guide: workstation, Tailscale, ssh, tmux, Claude Code, phone
- [docs/architecture.md](docs/architecture.md) — components, config, discovery, JSON contracts, extension points
- [docs/cli.md](docs/cli.md) — command reference
- [docs/desktop.md](docs/desktop.md) — Bearclaw.app, the web UI as a macOS app (Tauri)
- [docs/migration.md](docs/migration.md) — coming from `tb-bearclaw`, `ws` or `bearclaw-web`
- [web/](web/) — the web server and its HTTP API

## Security

There is **no authentication** in the web app. It is meant to be reachable only over your
tailnet: bind it to the Tailscale interface (or firewall it) and never expose it publicly. Anyone
who can reach the port can type into your Claude sessions (and read the notes under `web.notes.root`, when set). See
[docs/setup.md#security](docs/setup.md#security).

## License

MIT — see [LICENSE](LICENSE). Portions derived from
[productiveio/cli-toolbox](https://github.com/productiveio/cli-toolbox).
