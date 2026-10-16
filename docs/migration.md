# Migrating from tb-bearclaw, ws or bearclaw-web

`bearclaw` replaces three tools:

- **tb-bearclaw** — the Rust CLI that sees and steers Claude sessions on one machine;
- **ws** — a shell script that managed tmux sessions on a remote workstation over ssh;
- **bearclaw-web** — the Node web app that showed the bearclaw on a phone.

Behavior and JSON output are kept; names, config and env vars changed.

## Steps

1. Install `bearclaw` on every machine (`cargo install --path crates/bearclaw` on one,
   `bearclaw install --host <name>` for the others).
2. Run `bearclaw init` on each machine. It replaces the old per-tool config (bearclaw-web's
   `config.json` next to `server.mjs`, ws's host variable and machine detection).
3. Replace scripts and aliases using the tables below; rename `WS_*` / `TB_BEARCLAW_*` env vars.
4. Install the new skill and remove the old one: `bearclaw skill install`, then delete
   `~/.claude/skills/tb-bearclaw`.
5. Stop the old web server and start `bearclaw web serve` (or `bearclaw web install-service`).
6. Update tmux bindings that run `tb-bearclaw` (e.g. a `bearclaw` session running `tb-bearclaw watch`
   now runs `bearclaw watch`).
7. Remove the old binaries (`tb-bearclaw`, `ws`) and any shell function or alias named `ws` — an old
   shell function shadows a binary on `PATH` (check with `type ws`).

## tb-bearclaw → bearclaw

The session commands are identical; only the binary name changes. They now also accept `-H <host>`.

| tb-bearclaw | bearclaw |
| --- | --- |
| `tb-bearclaw` / `tb-bearclaw watch` | `bearclaw` / `bearclaw watch` |
| `tb-bearclaw list [--json]` | `bearclaw list [--json]` (+ `-a/--all-hosts`; rows now carry `host`) |
| `tb-bearclaw peek <t>` | `bearclaw peek <t>` |
| `tb-bearclaw send <t> <text>` | `bearclaw send <t> <text>` |
| `tb-bearclaw rename <t> <name>` | `bearclaw rename <t> <name>` |
| `tb-bearclaw name …` | `bearclaw name …` |
| `tb-bearclaw spawn …` | `bearclaw spawn …` |
| `tb-bearclaw handoff …` | `bearclaw handoff …` |
| `tb-bearclaw skill install` | `bearclaw skill install` |
| `TB_BEARCLAW_FIXTURE` | `BEARCLAW_FIXTURE` |

The `list --json` schema is unchanged except for the added `host` field. The naming cache (`~/.claude/bearclaw-names.json`) and handoff
briefs (`~/.claude/bearclaw-handoffs/`) keep their locations. Settings from tb-bearclaw's `config.toml` move into
`config.json` by hand: `[ui]` → `tui` (`rows`, `mouse`), `[naming]` → `naming` (`enabled`, `model`,
`syncTmux`, `autoTitle`), and `command` → `claude`. For example
`bearclaw config set naming.model haiku`.

Dropped: the toolbox version/update check.

## ws → bearclaw

`ws` ran commands on the workstation over ssh, or locally when run on it. `bearclaw` does the same
through the host model: tmux-session commands target `defaultHost` unless you pass `-H` or
`--local`, and "which machine am I" comes from `self` in the config instead of hostname or a
machine file.

| ws | bearclaw |
| --- | --- |
| `ws` / `ws list [-q]` | `bearclaw tmux` / `bearclaw tmux list [-q]` (alias `bearclaw t`) |
| `ws enter <query>` | `bearclaw enter <query>` |
| `ws last` | `bearclaw last` |
| `ws new [name] [-d] [-C dir] [-- cmd]` | `bearclaw new [name] [-d] [-C dir] [-- cmd]` |
| `ws kill <query> [-f]` | `bearclaw tmux kill <query> [-f]` |
| `ws rename <query> <name>` | `bearclaw tmux rename <query> <name>` |
| `ws stale [--kill] …` | `bearclaw tmux stale [--kill] …` |
| `ws f` / `ws bearclaw` | `bearclaw -H workstation list` |
| `ws f watch` | `bearclaw -H workstation watch` |
| `ws f <args…>` | `bearclaw -H workstation <args…>` |
| `ws f spawn …` (implied `--backend tmux`) | `bearclaw -H workstation spawn …` (tmux is the default over ssh) |
| `ws exec [-C dir] [-t] -- cmd` | `bearclaw exec [-C dir] [-t] -- cmd` |
| `ws ssh [cmd]` | `bearclaw ssh [cmd]` |
| `ws doctor` | `bearclaw doctor` |
| `ws --host <alias>` | `bearclaw -H <name>` (a config host name; its `ssh` field holds the alias) |
| `ws --local` / `--remote` | `bearclaw --local` / `bearclaw -H <name>` |
| `ws -n` | `bearclaw -n` / `BEARCLAW_DRY_RUN=1` |

Note that `bearclaw rename` is the *Claude* rename (formerly `ws f rename`); the tmux rename is
`bearclaw tmux rename`.

### Environment variables

| old | new |
| --- | --- |
| `WS_HOST` | `BEARCLAW_HOST` (a config host name, not an ssh alias) |
| `WS_LOCAL=1` | `--local` |
| `WS_DRY_RUN=1` | `BEARCLAW_DRY_RUN=1` |
| `WS_DEBUG=1` | `BEARCLAW_DEBUG=1` |
| `WS_CONNECT_TIMEOUT` | `BEARCLAW_CONNECT_TIMEOUT` |
| `WS_MUX=0` | `BEARCLAW_MUX=0` |
| `WS_BEARCLAW_TIMEOUT` | dropped — the stale cross-check is in-process now (`BEARCLAW_REMOTE_TIMEOUT` bounds remote calls) |
| `TB_BEARCLAW_FIXTURE` | `BEARCLAW_FIXTURE` |
| tb-bearclaw `command` config / wrapper | `BEARCLAW_CMD` or config `claude` |
| `NO_COLOR` | `NO_COLOR` |

## bearclaw-web → bearclaw web

The server and UI moved into `web/` and are started by the CLI.

| bearclaw-web | bearclaw |
| --- | --- |
| `node server.mjs` / `bin/dev.sh` | `bearclaw web serve` |
| its tmux / launchd start scripts | `bearclaw web install-service`, or a tmux session running `bearclaw web serve` |
| its deploy-to-workstation script | `bearclaw install --host workstation` |
| `config.json` next to `server.mjs` | `~/.config/bearclaw/config.json` (shared with the CLI) |
| `self` | `self` |
| `port`, `bind` | `web.port`, `web.bind` |
| `peers: { name: url }` | `hosts.<name>.web` |
| `tbBearclaw` | `bearclawBin` (optional; `bearclaw` is found on `PATH`) |
| `tmux` | `tmux` |
| quick-reply chips (hard-coded) | `web.quickReplies` |
| `autoName: { enabled, intervalMinutes }` | `web.autoName: { enabled, intervalMinutes }` (runs `bearclaw name --all --apply`) |
| `spawnDirs: [{ label, path }]` | `spawnDirs: [{ label, paths: { <host>: dir } }]` — one list for all hosts |
| `BEARCLAW_WEB_CONFIG` | `BEARCLAW_CONFIG` |
| `PORT` | `BEARCLAW_WEB_PORT` (`PORT` still works) |

The HTTP API is unchanged; session objects are `bearclaw list --json` objects plus `host`.
