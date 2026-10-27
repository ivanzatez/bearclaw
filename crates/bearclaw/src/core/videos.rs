//! Videos sessions made about their work: narrated scenes, one folder each under
//! `$BEARCLAW_VIDEOS_DIR`, else `${XDG_STATE_HOME:-~/.local/state}/bearclaw/videos`. The web app
//! plays them (its Videos tab, where you pause and ask the session by voice); the dashboard lists
//! them and opens one there. The folder's format is the web server's (web/ARCHITECTURE.md → Videos).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::core::config;

/// One video, as the dashboard lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Video {
    /// The folder's name: the web app's `#/videos/<id>`.
    pub id: String,
    pub title: String,
    /// The session that made it and answers questions about it.
    pub session: Option<String>,
    /// Seconds, the end of its last scene.
    pub duration: f64,
    pub scenes: usize,
    pub updated: Option<SystemTime>,
}

pub fn dir() -> PathBuf {
    if let Some(p) = std::env::var_os("BEARCLAW_VIDEOS_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/state"))
        .join("bearclaw/videos")
}

/// The web server's id rule: lower-case letters, digits and `-`, a letter or digit first.
fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// One folder's `video.json`, or `None` when it has none or it does not parse.
fn read(folder: &Path, id: &str) -> Option<Video> {
    let file = folder.join("video.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
    let scenes = v["scenes"].as_array().cloned().unwrap_or_default();
    let duration = scenes
        .iter()
        .map(|s| s["start"].as_f64().unwrap_or(0.0) + s["dur"].as_f64().unwrap_or(0.0))
        .fold(0.0, f64::max);
    Some(Video {
        id: id.to_string(),
        title: v["title"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or(id)
            .to_string(),
        session: v["session"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        duration,
        scenes: scenes.len(),
        updated: std::fs::metadata(&file).and_then(|m| m.modified()).ok(),
    })
}

/// Every video in `root`, newest first.
pub fn list_in(root: &Path) -> Vec<Video> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<Video> = entries
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            valid_id(&id).then(|| read(&e.path(), &id)).flatten()
        })
        .collect();
    out.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
    out
}

pub fn list() -> Vec<Video> {
    list_in(&dir())
}

/// Where the web app plays it: this machine's web server, on loopback (the browser is here).
pub fn url(id: &str) -> String {
    format!(
        "http://127.0.0.1:{}/#/videos/{id}",
        config::get().web_port()
    )
}

/// `147.5` → `2:27`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_with_a_video_json_newest_first() {
        let root = tempfile::tempdir().unwrap();
        let mk = |id: &str, json: &str| {
            std::fs::create_dir_all(root.path().join(id)).unwrap();
            std::fs::write(root.path().join(id).join("video.json"), json).unwrap();
        };
        mk(
            "subagents",
            r#"{"title":"Subagents","session":"abc","scenes":[{"start":0,"dur":10},{"start":10,"dur":12.5}]}"#,
        );
        mk("broken", "{not json");
        mk("Upper", r#"{"title":"skipped: not a valid id"}"#);
        mk("untitled", r#"{"scenes":[]}"#);
        std::fs::create_dir_all(root.path().join("empty")).unwrap();

        let list = list_in(root.path());
        let ids: Vec<&str> = list.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        let s = list.iter().find(|v| v.id == "subagents").unwrap();
        assert_eq!(
            (s.title.as_str(), s.session.as_deref(), s.duration, s.scenes),
            ("Subagents", Some("abc"), 22.5, 2)
        );
        let u = list.iter().find(|v| v.id == "untitled").unwrap();
        assert_eq!((u.title.as_str(), u.session.as_deref()), ("untitled", None));
        assert!(list_in(&root.path().join("nowhere")).is_empty());
    }

    #[test]
    fn clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(147.5), "2:27");
        assert_eq!(clock(-3.0), "0:00");
    }
}
