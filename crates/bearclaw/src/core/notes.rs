//! The notepad: a folder of your own text and markdown notes. Config `notes.root`, default
//! `~/notes`.

use std::path::PathBuf;

use crate::core::config;
use crate::core::tools::expand_tilde;

/// `notes.root` (read fresh, so a config edit applies), else `~/notes`.
pub fn root() -> PathBuf {
    let raw = config::load_from(&config::path()).raw;
    match raw["notes"]["root"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(r) => PathBuf::from(expand_tilde(r)),
        None => dirs::home_dir().unwrap_or_default().join("notes"),
    }
}
