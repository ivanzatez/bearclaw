//! QA sessions: a presenter session talks the user through someone's work, live and by voice,
//! and they interrupt it with questions. The web app runs them (its QA tab); each has a record in
//! `$BEARCLAW_QA_DIR`, else `${XDG_STATE_HOME:-~/.local/state}/bearclaw/qa/<id>/qa.json`
//! (web/lib/qa.mjs writes it). The dashboard lists them and opens one there.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::core::config;

/// One QA session, as the dashboard lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Qa {
    /// The folder's name: the web app's `#/qa/<id>`.
    pub id: String,
    pub title: String,
    /// The presenter's tmux session.
    pub presenter: String,
    /// The session whose work it presents, by title.
    pub source: Option<String>,
    pub lang: String,
    /// When it was started (ms since the epoch).
    pub created_at: Option<i64>,
}

pub fn dir() -> PathBuf {
    if let Some(p) = std::env::var_os("BEARCLAW_QA_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/state"))
        .join("bearclaw/qa")
}

/// The web server's id rule: lower-case letters, digits and `-`, a letter or digit first.
fn valid_id(id: &str) -> bool {
    id.len() <= 64
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn read(folder: &Path, id: &str) -> Option<Qa> {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("qa.json")).ok()?).ok()?;
    let text = |v: &Value| {
        v.as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    Some(Qa {
        id: id.to_string(),
        title: text(&v["title"]).unwrap_or_else(|| id.to_string()),
        presenter: text(&v["presenter"]["tmux"]).unwrap_or_else(|| format!("qa-{id}")),
        source: text(&v["source"]["title"]),
        lang: text(&v["lang"]).unwrap_or_else(|| "hr".into()),
        created_at: v["createdAt"].as_i64(),
    })
}

/// Every QA session in `root`, newest first.
pub fn list_in(root: &Path) -> Vec<Qa> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<Qa> = entries
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            valid_id(&id).then(|| read(&e.path(), &id)).flatten()
        })
        .collect();
    out.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

pub fn list() -> Vec<Qa> {
    list_in(&dir())
}

/// Where the web app runs it (`None`: the QA list, to start a new one). This machine's web
/// server, on loopback: the browser is here.
pub fn url(id: Option<&str>) -> String {
    let port = config::get().web_port();
    match id {
        Some(id) => format!("http://127.0.0.1:{port}/#/qa/{id}"),
        None => format!("http://127.0.0.1:{port}/#/qa"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_qa_sessions_newest_first() {
        let root = tempfile::tempdir().unwrap();
        let mk = |id: &str, json: &str| {
            std::fs::create_dir_all(root.path().join(id)).unwrap();
            std::fs::write(root.path().join(id).join("qa.json"), json).unwrap();
        };
        mk(
            "subagents-ab12",
            r#"{"title":"Subagents","presenter":{"tmux":"qa-subagents-ab12"},"source":{"title":"agent-orchestration-subagent"},"lang":"en","createdAt":2}"#,
        );
        mk("older-0000", r#"{"title":"Older","createdAt":1}"#);
        mk("broken", "{nope");
        mk("Upper", r#"{"title":"not a valid id"}"#);

        let list = list_in(root.path());
        assert_eq!(
            list.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(),
            ["subagents-ab12", "older-0000"]
        );
        assert_eq!(
            (
                list[0].presenter.as_str(),
                list[0].source.as_deref(),
                list[0].lang.as_str()
            ),
            (
                "qa-subagents-ab12",
                Some("agent-orchestration-subagent"),
                "en"
            )
        );
        assert_eq!(
            (
                list[1].presenter.as_str(),
                list[1].source.as_deref(),
                list[1].lang.as_str()
            ),
            ("qa-older-0000", None, "hr")
        );
        assert!(list_in(&root.path().join("nowhere")).is_empty());
    }
}
