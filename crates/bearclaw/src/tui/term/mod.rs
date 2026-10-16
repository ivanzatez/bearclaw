//! A tmux session drawn inside the dashboard: keys go in as xterm bytes ([`keys`]), the screen
//! comes back through a PTY and a terminal emulator ([`embed`]).

pub mod embed;
pub mod keys;
