//! The todo: one-line work items, kept up to date by Claude sessions (their own item) and by
//! you. One JSON file; every change is a locked read-modify-write, since many sessions write.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// How long an item's text can be: one line in the Todo tab.
pub const TEXT_MAX: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Nobody is on it yet.
    Open,
    /// A session (or you) is working on it.
    Doing,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: u64,
    pub text: String,
    pub status: Status,
    /// The Claude session id working on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// That session's name when it took the item — still readable after it ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_url: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub items: Vec<Item>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// One line, at most `max` characters: newlines and runs of whitespace become one space.
pub fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

impl Todo {
    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Result<&mut Item> {
        self.items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| Error::Other(format!("no todo item #{id}")))
    }

    /// The item a session is working on right now.
    pub fn doing_for(&self, session: &str) -> Option<&Item> {
        self.items
            .iter()
            .find(|i| i.status == Status::Doing && i.session.as_deref() == Some(session))
    }

    /// A new item: Doing when a session `(id, label)` takes it on, else Open.
    pub fn add(
        &mut self,
        text: &str,
        task_url: Option<&str>,
        session: Option<(&str, &str)>,
    ) -> u64 {
        self.next_id = self
            .next_id
            .max(self.items.iter().map(|i| i.id).max().unwrap_or(0))
            + 1;
        let now = now_ms();
        self.items.push(Item {
            id: self.next_id,
            text: one_line(text, TEXT_MAX),
            status: if session.is_some() {
                Status::Doing
            } else {
                Status::Open
            },
            session: session.map(|(id, _)| id.to_string()),
            session_label: session.map(|(_, label)| label.to_string()),
            task_url: task_url.map(str::to_string),
            created_ms: now,
            updated_ms: now,
        });
        self.next_id
    }

    /// A session's own item: updated when it has one in progress, else added. A task link
    /// already on the item stays unless a new one is given.
    pub fn set_for(
        &mut self,
        session: &str,
        label: &str,
        text: &str,
        task_url: Option<&str>,
    ) -> u64 {
        let Some(id) = self.doing_for(session).map(|i| i.id) else {
            return self.add(text, task_url, Some((session, label)));
        };
        let item = self.get_mut(id).expect("just found");
        item.text = one_line(text, TEXT_MAX);
        item.session_label = Some(label.to_string());
        if let Some(t) = task_url {
            item.task_url = Some(t.to_string());
        }
        item.updated_ms = now_ms();
        id
    }

    /// A session takes an item on (one you added, say).
    pub fn claim(&mut self, id: u64, session: &str, label: &str) -> Result<()> {
        let item = self.get_mut(id)?;
        item.status = Status::Doing;
        item.session = Some(session.to_string());
        item.session_label = Some(label.to_string());
        item.updated_ms = now_ms();
        Ok(())
    }

    pub fn done(&mut self, id: u64) -> Result<()> {
        let item = self.get_mut(id)?;
        item.status = Status::Done;
        item.updated_ms = now_ms();
        Ok(())
    }

    /// Back to work: Doing when it has a session, else Open.
    pub fn reopen(&mut self, id: u64) -> Result<()> {
        let item = self.get_mut(id)?;
        item.status = if item.session.is_some() {
            Status::Doing
        } else {
            Status::Open
        };
        item.updated_ms = now_ms();
        Ok(())
    }

    pub fn remove(&mut self, id: u64) -> Result<()> {
        let before = self.items.len();
        self.items.retain(|i| i.id != id);
        if self.items.len() == before {
            return Err(Error::Other(format!("no todo item #{id}")));
        }
        Ok(())
    }
}

/// `$BEARCLAW_TODO_FILE`, else `${XDG_STATE_HOME:-~/.local/state}/bearclaw/todo.json`.
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("BEARCLAW_TODO_FILE").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/state"))
        .join("bearclaw/todo.json")
}

/// A missing file is an empty todo; one that doesn't parse is an error (and stays untouched).
pub fn load(path: &Path) -> Result<Todo> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| Error::Other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Todo::default()),
        Err(e) => Err(e.into()),
    }
}

/// Lock, load, change, save. `f`'s error leaves the file as it was.
pub fn update<T>(path: &Path, f: impl FnOnce(&mut Todo) -> Result<T>) -> Result<T> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    let guard = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(PathBuf::from(lock))?;
    guard.lock()?;
    let mut todo = load(path)?;
    let out = f(&mut todo)?;
    todo.version = 1;
    crate::core::brief::write_private(path, &(serde_json::to_string_pretty(&todo)? + "\n"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_move_through_their_states() {
        let mut t = Todo::default();
        let mine = t.add("Look at Tini's PR", None, None);
        let s = t.set_for(
            "sess-a",
            "work-2",
            "Permission error\nin the loader",
            Some("https://app.example/tasks/7"),
        );
        assert_eq!(t.items.len(), 2);
        let item = t.get(s).unwrap();
        assert_eq!(item.text, "Permission error in the loader");
        assert_eq!(item.status, Status::Doing);
        assert_eq!(item.session.as_deref(), Some("sess-a"));
        assert_eq!(
            item.task_url.as_deref(),
            Some("https://app.example/tasks/7")
        );
        // Setting again updates the session's item rather than adding one.
        let again = t.set_for("sess-a", "work-2", "Permission error fixed, testing", None);
        assert_eq!(again, s);
        assert_eq!(t.items.len(), 2);
        assert_eq!(
            t.get(s).unwrap().task_url.as_deref(),
            Some("https://app.example/tasks/7")
        );
        assert_eq!(t.doing_for("sess-a").map(|i| i.id), Some(s));

        t.claim(mine, "sess-b", "api-fix").unwrap();
        assert_eq!(t.get(mine).unwrap().status, Status::Doing);
        t.done(s).unwrap();
        assert_eq!(t.get(s).unwrap().status, Status::Done);
        assert!(t.doing_for("sess-a").is_none());
        // A finished session starting on something new gets a new item.
        let next = t.set_for("sess-a", "work-2", "Next thing", None);
        assert_ne!(next, s);
        t.reopen(s).unwrap();
        assert_eq!(t.get(s).unwrap().status, Status::Doing);
        t.remove(mine).unwrap();
        assert!(t.get(mine).is_none());
        assert!(t.done(999).is_err());
    }

    #[test]
    fn one_line_folds_and_caps() {
        assert_eq!(one_line("  a\n  b\r\nc  ", 50), "a b c");
        assert_eq!(one_line("abcdef", 4), "abc…");
        assert_eq!(one_line("", 10), "");
    }

    #[test]
    fn concurrent_writers_lose_nothing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("todo.json");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    update(&p, |t| {
                        t.add(&format!("item {i}"), None, None);
                        Ok(())
                    })
                    .unwrap()
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let t = load(&p).unwrap();
        assert_eq!(t.items.len(), 8);
        let mut ids: Vec<u64> = t.items.iter().map(|i| i.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 8);
    }

    #[test]
    fn missing_is_empty_and_corrupt_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        assert!(load(&d.path().join("none.json")).unwrap().items.is_empty());
        let bad = d.path().join("bad.json");
        std::fs::write(&bad, "{ nope").unwrap();
        assert!(load(&bad).is_err());
        assert!(
            update(&bad, |t| {
                t.add("x", None, None);
                Ok(())
            })
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ nope");
    }
}
