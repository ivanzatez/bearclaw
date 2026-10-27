//! The reusable core. Nothing in here prints to stdout on its own (dry-run
//! output aside) — rendering belongs to the UIs.

pub mod backend;
pub mod brain;
pub mod brief;
pub mod claw;
pub mod config;
pub mod context;
pub mod discovery;
pub mod grouping;
pub mod harness;
pub mod hosts;
pub mod jobs;
pub mod naming;
pub mod notes;
pub mod productive;
pub mod prs;
pub mod qa;
pub mod repos;
pub mod snapshot;
pub mod stack;
pub mod sticky;
pub mod tbpr;
pub mod title;
pub mod tmux;
pub mod todo;
pub mod tools;
pub mod usage;
pub mod webapi;
