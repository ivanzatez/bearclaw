//! `bearclaw todo …` and `bearclaw sticky …` — what a Claude session runs to keep its todo item
//! and sticky note current; the calling session is found from the process tree.

use crate::core::discovery::{self, Session};
use crate::core::sticky;
use crate::core::todo::{self, Status};
use crate::error::{Error, Result};

#[derive(clap::Subcommand, Debug, Clone)]
pub enum TodoAction {
    /// List the items (the default)
    List {
        #[arg(long)]
        json: bool,
    },
    /// Add an item nobody is on yet
    Add {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        /// A Productive task link
        #[arg(long)]
        task: Option<String>,
    },
    /// The calling session's own item, in one line: created, or updated when it has one
    Set {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[arg(long)]
        task: Option<String>,
        /// The session (default: the Claude session running this command)
        #[arg(long)]
        session: Option<String>,
    },
    /// The calling session takes item ID
    Claim {
        id: u64,
        #[arg(long)]
        session: Option<String>,
    },
    /// Mark an item done (default: the calling session's item)
    Done {
        id: Option<u64>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Remove an item
    Rm { id: u64 },
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum StickyAction {
    /// Replace the calling session's note: one bullet per argument, at most six
    Set {
        #[arg(required = true, num_args = 1..)]
        bullets: Vec<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Print a session's note (default: the calling session's)
    Show {
        target: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

/// `--session <target>`, else the Claude session this command runs inside.
fn calling(session: Option<&str>) -> Result<(String, String)> {
    let s: Session = match session {
        Some(t) => discovery::resolve(t).map_err(Error::Other)?,
        None => discovery::origin().ok_or_else(|| {
            Error::Other(
                "not inside a supervised Claude session — skip the todo and sticky-note steps \
                 (a person can pass --session <target>, or use `bearclaw todo add`)"
                    .into(),
            )
        })?,
    };
    let id = s
        .session_id
        .clone()
        .ok_or_else(|| Error::Other(format!("{} has no session id yet", s.label())))?;
    Ok((id, s.headline()))
}

fn mark(status: Status) -> &'static str {
    match status {
        Status::Doing => "●",
        Status::Open => "○",
        Status::Done => "✓",
    }
}

/// The item's text, one line and not empty — and its task link has to be a web link.
fn checked(text: &[String], task: Option<&str>) -> Result<String> {
    let line = todo::one_line(&text.join(" "), todo::TEXT_MAX);
    if line.is_empty() {
        return Err(Error::Other("the item needs some text".into()));
    }
    if let Some(t) = task
        && !todo::task_url_ok(t)
    {
        return Err(Error::Other(format!(
            "--task must be an http(s) link: {t:?}"
        )));
    }
    Ok(line)
}

pub fn run_todo(action: Option<TodoAction>) -> Result<()> {
    let path = todo::path();
    match action.unwrap_or(TodoAction::List { json: false }) {
        TodoAction::List { json } => {
            let t = todo::load(&path)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&t)?);
                return Ok(());
            }
            for i in &t.items {
                let who = i
                    .session_label
                    .as_deref()
                    .map(|l| format!("  ⧉ {l}"))
                    .unwrap_or_default();
                let task = i
                    .task_url
                    .as_deref()
                    .map(|u| format!("  ↗ {u}"))
                    .unwrap_or_default();
                println!("{} #{} {}{who}{task}", mark(i.status), i.id, i.text);
            }
            Ok(())
        }
        TodoAction::Add { text, task } => {
            let line = checked(&text, task.as_deref())?;
            let id = todo::update(&path, |t| Ok(t.add(&line, task.as_deref(), None)))?;
            println!("#{id}");
            Ok(())
        }
        TodoAction::Set {
            text,
            task,
            session,
        } => {
            let line = checked(&text, task.as_deref())?;
            let (sid, label) = calling(session.as_deref())?;
            let id = todo::update(&path, |t| {
                Ok(t.set_for(&sid, &label, &line, task.as_deref()))
            })?;
            println!("#{id}");
            Ok(())
        }
        TodoAction::Claim { id, session } => {
            let (sid, label) = calling(session.as_deref())?;
            todo::update(&path, |t| t.claim(id, &sid, &label))
        }
        TodoAction::Done { id, session } => {
            let id = match id {
                Some(id) => id,
                None => {
                    let (sid, _) = calling(session.as_deref())?;
                    todo::load(&path)?
                        .doing_for(&sid)
                        .map(|i| i.id)
                        .ok_or_else(|| {
                            Error::Other("this session has no item in progress".into())
                        })?
                }
            };
            todo::update(&path, |t| t.done(id))
        }
        TodoAction::Rm { id } => todo::update(&path, |t| t.remove(id)),
    }
}

pub fn run_sticky(action: StickyAction) -> Result<()> {
    let dir = sticky::dir();
    match action {
        StickyAction::Set { bullets, session } => {
            let (sid, _) = calling(session.as_deref())?;
            for b in sticky::set(&dir, &sid, &bullets)? {
                println!("- {b}");
            }
            Ok(())
        }
        StickyAction::Show { target, json } => {
            let (sid, _) = calling(target.as_deref())?;
            let note = sticky::get(&dir, &sid);
            if json {
                let v = serde_json::json!({
                    "session": sid,
                    "bullets": note.as_ref().map(|n| n.bullets.clone()).unwrap_or_default(),
                    "updatedMs": note.as_ref().map(|n| n.updated_ms),
                });
                println!("{v}");
            } else {
                for b in note.map(|n| n.bullets).unwrap_or_default() {
                    println!("- {b}");
                }
            }
            Ok(())
        }
    }
}
