//! `bearclaw claw start|path` — Claw, the operator session behind the dashboard's Claw tab, for
//! other front ends (the web app's Claw tab).

use crate::core::claw;
use crate::error::{Error, Result};

#[derive(clap::Subcommand, Debug, Clone)]
pub enum ClawAction {
    /// Find Claw's tmux session, or start it in its workspace; prints the tmux session
    Start {
        /// `{ "session": <tmux session>, "dir": <workspace> }`
        #[arg(long)]
        json: bool,
    },
    /// Print Claw's workspace (the session runs there)
    Path,
}

pub fn run(action: &ClawAction) -> Result<()> {
    match action {
        ClawAction::Start { json } => {
            let session = claw::find_or_start().map_err(Error::Other)?;
            if *json {
                let out = serde_json::json!({ "session": session, "dir": claw::dir() });
                println!("{out}");
            } else {
                println!("{session}");
            }
            Ok(())
        }
        ClawAction::Path => {
            println!("{}", claw::dir().display());
            Ok(())
        }
    }
}
