//! A session's sticky note: up to six one-line bullets — what it's doing, what it has done —
//! written by the session itself, shown next to it on the dashboard.

use std::path::{Path, PathBuf};

use crate::core::todo::one_line;
use crate::error::{Error, Result};

pub const MAX_BULLETS: usize = 6;
const BULLET_MAX: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sticky {
    pub bullets: Vec<String>,
    pub updated_ms: i64,
}

/// `$BEARCLAW_STICKY_DIR`, else `${XDG_STATE_HOME:-~/.local/state}/bearclaw/sticky`.
pub fn dir() -> PathBuf {
    if let Some(p) = std::env::var_os("BEARCLAW_STICKY_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/state"))
        .join("bearclaw/sticky")
}

fn file(dir: &Path, session_id: &str) -> Result<PathBuf> {
    let ok = !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(Error::Other(format!("not a session id: {session_id:?}")));
    }
    Ok(dir.join(format!("{session_id}.md")))
}

/// Replace the note: bullets one-lined (a leading `-`/`*` dropped), empties skipped, at most
/// [`MAX_BULLETS`]. Returns what was kept.
pub fn set(dir: &Path, session_id: &str, bullets: &[String]) -> Result<Vec<String>> {
    let path = file(dir, session_id)?;
    let kept: Vec<String> = bullets
        .iter()
        .map(|b| {
            let b = b.trim_start();
            let b = b.strip_prefix("- ").or(b.strip_prefix("* ")).unwrap_or(b);
            one_line(b, BULLET_MAX)
        })
        .filter(|b| !b.is_empty())
        .take(MAX_BULLETS)
        .collect();
    let body: String = kept.iter().map(|b| format!("- {b}\n")).collect();
    crate::core::brief::write_private(&path, &body)?;
    Ok(kept)
}

pub fn get(dir: &Path, session_id: &str) -> Option<Sticky> {
    let path = file(dir, session_id).ok()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let updated_ms = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64);
    let bullets = text
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .map(str::to_string)
        .collect();
    Some(Sticky {
        bullets,
        updated_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bullets_are_one_line_and_at_most_six() {
        let d = tempfile::tempdir().unwrap();
        let input: Vec<String> = [
            "- Found the 403\n in the loader",
            "",
            "  ",
            "b",
            "c",
            "d",
            "e",
            "f",
            "g",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let kept = set(d.path(), "sess-a", &input).unwrap();
        assert_eq!(kept.len(), MAX_BULLETS);
        assert_eq!(kept[0], "Found the 403 in the loader");
        let s = get(d.path(), "sess-a").unwrap();
        assert_eq!(s.bullets, kept);
        assert!(s.updated_ms > 0);
        assert!(get(d.path(), "nobody").is_none());
        // Ids come from Claude, but never let one walk out of the folder.
        assert!(set(d.path(), "../x", &input).is_err());
    }
}
