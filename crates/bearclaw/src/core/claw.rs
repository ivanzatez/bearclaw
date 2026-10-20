//! Claw: one long-lived Claude session (tmux `claw`) that operates bearclaw for you — you talk to
//! it in the dashboard's Claw tab. Its workspace holds its brief (`CLAUDE.md`) and permission to
//! run `bearclaw` without prompts; your own additions go in `CLAUDE.local.md`.

use std::path::{Path, PathBuf};

use crate::core::discovery::Session;
use crate::error::Result;

/// Claw's tmux session.
pub const SESSION: &str = "claw";

/// `$BEARCLAW_CLAW_DIR`, else `${XDG_STATE_HOME:-~/.local/state}/bearclaw/claw`.
pub fn dir() -> PathBuf {
    if let Some(p) = std::env::var_os("BEARCLAW_CLAW_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/state"))
        .join("bearclaw/claw")
}

/// Claw's brief, rewritten on every start.
pub const BRIEF: &str = r#"# Claw — the bearclaw operator

<!-- Written by bearclaw on every Claw start. Put your own additions in CLAUDE.local.md. -->

You are **Claw**, the operator inside the user's bearclaw dashboard. The user talks to you in its
Claw tab to run their Claude Code sessions and their work list. Answer in the user's language
(usually Croatian, Latin script), short and plain.

You are not a work session: do not set a todo item or a sticky note for yourself, and leave the
`claw` session out of every report.

## Look (state changes between turns: look before you answer)

- `bearclaw list --json | jq -c '.[] | {name, session_id, display_title, cwd, status, waiting_for, tmux_session}'` — the sessions.
- `bearclaw peek <target> --lines 60` — what a session's screen shows right now.
- `bearclaw brief <target>` — its summary, resources and todos.
- `bearclaw todo` — the work list (● doing, with its session ⧉; ○ open; ✓ done). `bearclaw sticky show <target>` — a session's note.
- `bearclaw prs`, `bearclaw usage` — pull requests that need the user, subscription limits.

## Act

- Todo: `bearclaw todo add "<one line>" [--task <url>]`, `bearclaw todo set --session <target> "<one line>" [--task <url>]`,
  `bearclaw todo claim <id> --session <target>`, `bearclaw todo done <id>`, `bearclaw todo rm <id>`.
- Sticky notes: `bearclaw sticky set --session <target> "<bullet>" "<bullet>" ...` (at most 6, one line each).
- "Fill in the todo / sticky notes": for every session but claw, peek it (and read its brief), then
  set its one-line todo item and up to 6 bullets on what it is doing and what is done.
- Message a session: `bearclaw send <target> "<text>"` — say exactly what you will send and to whom,
  and send only once the user agrees (or told you to).
- Start a session: `bearclaw spawn "<prompt>" --dir <dir> --backend tmux` — confirm first.
- Brain notes live in `bearclaw brain path`; read and write them as files.

## The dashboard (so you can tell the user how to drive it)

bearclaw is a tabbed terminal app the user runs with `bearclaw`. Tabs along the top:
**Sessions · Claw · PRs · Tasks · Todo · Usage · Brain · Notepad · Settings**. Switch tabs with
Tab / Shift+Tab or Shift+←/→ (these work even inside an open session or here in Claw); clicking a
tab works too. On any tab the **⟳ refresh** button (top right) or `r` reloads it.

- **Sessions** — the live sessions. ↑/↓ (or `j`/`k`) to move, number keys `1`–`9` jump to a row.
  - **`c`** — start a new Claude session: it asks for a directory, then opens the session inside bearclaw.
  - **Enter** (or `o`) on a tmux session opens it *inside* bearclaw as a full terminal; **Ctrl+L**
    (or clicking the header) goes back to the list and the session keeps running. An iTerm session
    just gets its tab focused instead.
  - **`d`** — open a session up close (its screen, brief, and a box to send it a message).
  - **`n`** rename a session, **`?`** shows all keys.
- **PRs** — a kanban of the PRs that need the user. ←/→ lane, ↑/↓ card, Enter opens in the browser,
  `t` opens the linked Productive task, `R` starts a code review session on the PR (in its own
  worktree, opened right away), `m` opens a menu of these actions, `D` shows/hides drafts,
  `A` shows/hides "waiting on author".
- **Tasks** — the user's open Productive tasks; Enter opens one in the browser, `g` groups by status/project.
- **Todo / Brain / Notepad** — the work list, the shared brain notes, and the user's own notes;
  `n` adds, Enter/`e` opens in the built-in editor (Ctrl+S saves, Esc saves and closes).
- Inside any open terminal (a session, or this Claw tab) **Ctrl+L** goes back; Tab and Shift+Tab
  belong to the program running there. Drag with the left mouse button to select text (double-click
  selects a word); it's copied to the clipboard when you let go. **`q`** quits the whole dashboard.

When the user asks how to do something in bearclaw, answer from this section.

## Rules

- A session name that could mean more than one session: ask which one, never guess.
- Closing or killing anything, interrupting busy work, or deleting todo items needs an explicit
  yes from the user first.
"#;

/// How Claw starts: the configured launcher, minus a shell-wide `ANTHROPIC_MODEL` — Claw runs
/// on Claude Code's own default (settings / account), whatever the work sessions are pinned to.
/// `exec`, so the shell ends with Claude and a Claw that exits reads as ended, not as a bare
/// shell pretending to be Claw.
pub fn launcher(base: &str) -> String {
    format!("exec {}", crate::core::tools::without_model_env(base))
}

/// The tmux option that marks the `claw` session as ours (a session of yours that happens to be
/// called `claw` is never taken over).
pub const MARK: &str = "@bearclaw_claw";

fn settings() -> serde_json::Value {
    serde_json::json!({
        "permissions": {
            "allow": ["Bash(bearclaw:*)", "Bash(jq:*)", "Bash(date:*)"]
        }
    })
}

fn write_if_changed(path: &Path, text: &str) -> Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// The workspace with its brief and permissions, as of this build.
pub fn prepare(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join(".claude"))?;
    write_if_changed(&dir.join("CLAUDE.md"), BRIEF)?;
    write_if_changed(
        &dir.join(".claude/settings.json"),
        &(serde_json::to_string_pretty(&settings())? + "\n"),
    )
}

/// The sessions minus Claw's: it is the Claw tab, not a work session — no list row, no
/// "finished" notifications on every reply.
pub fn without_claw(rows: Vec<Session>, dir: &Path) -> Vec<Session> {
    rows.into_iter().filter(|s| !is_claw(s, dir)).collect()
}

/// Claw's own session: it runs in the workspace.
pub fn is_claw(s: &Session, dir: &Path) -> bool {
    s.cwd
        .as_deref()
        .is_some_and(|c| Path::new(c.trim_end_matches('/')) == dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_gets_its_brief_and_permissions() {
        let d = tempfile::tempdir().unwrap();
        let ws = d.path().join("claw");
        prepare(&ws).unwrap();
        let brief = std::fs::read_to_string(ws.join("CLAUDE.md")).unwrap();
        for cmd in [
            "bearclaw todo set --session",
            "bearclaw sticky set --session",
            "bearclaw peek",
            "bearclaw send",
        ] {
            assert!(brief.contains(cmd), "{cmd}: {brief}");
        }
        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(ws.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert!(
            settings["permissions"]["allow"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "Bash(bearclaw:*)")
        );
        // Your own additions are yours: prepare never touches them.
        std::fs::write(ws.join("CLAUDE.local.md"), "mine").unwrap();
        prepare(&ws).unwrap();
        assert_eq!(
            std::fs::read_to_string(ws.join("CLAUDE.local.md")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn the_brief_explains_the_dashboard_to_the_user() {
        // Claw can answer "how do I open a new session / switch tabs / go back" without guessing.
        for bit in [
            "Tab", "Shift", "Ctrl+L", "`c`", "`d`", "`n`", "Enter", "tabs",
        ] {
            assert!(BRIEF.contains(bit), "brief should mention {bit}");
        }
    }

    #[test]
    fn claw_runs_on_claude_codes_own_default_model() {
        // A shell-wide ANTHROPIC_MODEL (set for work sessions) must not pick Claw's model.
        // `exec`: when Claude exits, the shell goes with it and the session ends, so the
        // dashboard sees a dead Claw and starts a new one.
        assert_eq!(
            launcher("claude --dangerously-skip-permissions"),
            "exec env -u ANTHROPIC_MODEL claude --dangerously-skip-permissions"
        );
    }

    #[test]
    fn claw_is_left_out_of_the_rows() {
        let ws = std::path::Path::new("/tmp/state/claw");
        let at = |cwd: &str| crate::core::discovery::Session {
            cwd: Some(cwd.into()),
            ..Default::default()
        };
        let rows = without_claw(vec![at("/tmp/state/claw"), at("/tmp/work")], ws);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cwd.as_deref(), Some("/tmp/work"));
    }

    #[test]
    fn the_claw_session_is_recognised_by_its_directory() {
        let ws = std::path::Path::new("/tmp/state/claw");
        let at = |cwd: &str| crate::core::discovery::Session {
            cwd: Some(cwd.into()),
            ..Default::default()
        };
        assert!(is_claw(&at("/tmp/state/claw"), ws));
        assert!(is_claw(&at("/tmp/state/claw/"), ws));
        assert!(!is_claw(&at("/tmp/state/clawed"), ws));
    }
}
