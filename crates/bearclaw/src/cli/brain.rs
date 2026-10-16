//! `bearclaw brain hook|install|path` — the brain's wiring into Claude Code.

use std::path::Path;

use serde_json::Value;

use crate::core::brain;
use crate::core::discovery::claude_home;
use crate::core::tools::shq_min;
use crate::error::{Error, Result};

#[derive(clap::Subcommand, Debug, Clone)]
pub enum BrainAction {
    /// SessionStart hook: print the brain's index for Claude (never fails)
    Hook,
    /// Create the brain folder and add the hook to ~/.claude/settings.json
    Install {
        /// Remove the hook instead
        #[arg(long)]
        uninstall: bool,
    },
    /// Print the brain folder
    Path,
}

pub fn run(action: &BrainAction) -> Result<()> {
    match action {
        BrainAction::Hook => {
            hook();
            Ok(())
        }
        BrainAction::Install { uninstall } => install(*uninstall),
        BrainAction::Path => {
            println!("{}", brain::root().display());
            Ok(())
        }
    }
}

/// Runs at the start of every Claude session, so it prints nothing rather than fail: a brain
/// that isn't there (or can't be read) must not get in a session's way.
fn hook() {
    let root = brain::root();
    if !root.is_dir() {
        return;
    }
    let notes = brain::list(&root);
    let out = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": brain::hook_context(&root, &notes, 100),
        }
    });
    println!("{out}");
}

fn install(uninstall: bool) -> Result<()> {
    let path = claude_home().join("settings.json");
    let mut settings: Value = match std::fs::read_to_string(&path) {
        Ok(text) if text.trim().is_empty() => Value::Object(Default::default()),
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            Error::Other(format!(
                "{} is not valid JSON ({e}) — left untouched",
                path.display()
            ))
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Default::default()),
        Err(e) => return Err(e.into()),
    };
    let changed = if uninstall {
        brain::uninstall_hook(&mut settings)
    } else {
        let root = brain::root();
        if !root.is_dir() {
            std::fs::create_dir_all(&root)?;
            println!("Created the brain: {}", root.display());
        } else {
            println!("Brain: {}", root.display());
        }
        let exe = crate::cli::hosts::current_exe()?;
        let command = format!("{} brain hook", shq_min(&exe.display().to_string()));
        brain::install_hook(&mut settings, &command)
            .map_err(|e| Error::Other(format!("{}: {e} — left untouched", path.display())))?
    };
    if !changed {
        println!("{}: nothing to change", path.display());
        return Ok(());
    }
    write_atomic(&path, &(serde_json::to_string_pretty(&settings)? + "\n"))?;
    println!(
        "{} the SessionStart hook {} {}",
        if uninstall { "Removed" } else { "Added" },
        if uninstall { "from" } else { "to" },
        path.display()
    );
    Ok(())
}

/// Through a symlink (dotfiles keep settings.json as one) onto the file behind it, with that
/// file's permissions.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = target.with_extension("json.bearclaw-tmp");
    std::fs::write(&tmp, text)?;
    if let Ok(meta) = std::fs::metadata(&target) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, &target)?;
    Ok(())
}
