//! `bearclaw tasks` — your open Productive tasks (the dashboard's Tasks tab), sorted like it.

use serde_json::json;

use crate::core::productive::{self, Settings, Task};
use crate::error::{Error, Result};

pub fn run(json: bool) -> Result<()> {
    let settings = Settings::load().map_err(Error::Other)?;
    let tasks = productive::my_open_tasks(&settings).map_err(Error::Other)?;
    if json {
        let rows: Vec<_> = tasks.iter().map(row).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "tasks": rows }))?
        );
    } else {
        for t in &tasks {
            println!(
                "{:>8}  {:<14}  {}  ({})",
                t.number, t.status, t.title, t.project
            );
        }
    }
    Ok(())
}

fn row(t: &Task) -> serde_json::Value {
    json!({
        "id": t.id,
        "number": t.number,
        "title": t.title,
        "project": t.project,
        "status": t.status,
        "dueDate": t.due_date,
        "updatedAt": t.updated_at,
        "url": t.url,
        "description": t.description,
        "statusCategory": t.status_rank.0,
    })
}
