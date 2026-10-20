//! A tmux client in a PTY, its output run through a terminal emulator and drawn into the
//! dashboard's buffer: the session looks and behaves as it would in its own terminal.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::core::tools::tmux_cmd;

pub struct Embedded {
    pub(crate) parser: vt100::Parser,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    /// The reader hit EOF: the client (or its session) is gone.
    ended: bool,
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Embedded {
    /// Attach to tmux session `target` on the default server, set up first with
    /// [`session_options`].
    pub fn attach(target: &str, rows: u16, cols: u16) -> Result<Embedded, String> {
        for args in session_options(target) {
            let _ = tmux_cmd()
                .args(&args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        let mut cmd = CommandBuilder::new(crate::core::tools::tmux());
        cmd.args(["-u", "attach-session", "-t", &format!("={target}")]);
        Self::attach_cmd(cmd, rows, cols)
    }

    /// Run `cmd` (a tmux client) in a new PTY of `rows`×`cols`.
    pub fn attach_cmd(mut cmd: CommandBuilder, rows: u16, cols: u16) -> Result<Embedded, String> {
        cmd.env("TERM", "xterm-256color");
        // A client started from inside tmux would refuse to nest.
        cmd.env_remove("TMUX");
        let pair = native_pty_system()
            .openpty(size(rows, cols))
            .map_err(|e| format!("pty: {e}"))?;
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("tmux: {e}"))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("pty: {e}"))?;
        let writer = pair.master.take_writer().map_err(|e| format!("pty: {e}"))?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 16 * 1024];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 || tx.send(chunk[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(Embedded {
            parser: vt100::Parser::new(rows.max(1), cols.max(1), 0),
            master: pair.master,
            writer,
            child,
            rx,
            ended: false,
        })
    }

    /// Feed what the client printed into the emulator. True when the screen changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(bytes) => {
                    self.parser.process(&bytes);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    changed |= !self.ended;
                    self.ended = true;
                    break;
                }
            }
        }
        changed
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if self.writer.write_all(bytes).is_err() || self.writer.flush().is_err() {
            self.ended = true;
        }
    }

    /// Match the PTY (and so the tmux client) to the area it is drawn in.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (r, c) = self.parser.screen().size();
        if (r, c) == (rows.max(1), cols.max(1)) {
            return;
        }
        self.parser.set_size(rows.max(1), cols.max(1));
        let _ = self.master.resize(size(rows, cols));
    }

    pub fn alive(&mut self) -> bool {
        !self.ended && matches!(self.child.try_wait(), Ok(None))
    }

    pub fn draw(&self, buf: &mut Buffer, area: Rect) {
        draw_screen(self.parser.screen(), buf, area);
    }

    /// The emulator's current screen — its cell grid, for reading selected text.
    pub(crate) fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
}

impl Drop for Embedded {
    /// Detaching only: the session lives on in tmux.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `tmux set-option` calls that hand a session to the dashboard: no status bar, no
/// prefix key (every key belongs to the program inside), mouse on for wheel scrolling.
pub fn session_options(target: &str) -> Vec<Vec<String>> {
    // `=name` alone is "no such session" here; the exact-match form takes a trailing colon.
    let t = format!("={target}:");
    [("status", "off"), ("prefix", "None"), ("mouse", "on")]
        .into_iter()
        .map(|(opt, val)| {
            ["set-option", "-t", &t, opt, val]
                .into_iter()
                .map(str::to_string)
                .collect()
        })
        .collect()
}

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// The emulator's screen into `area` of `buf`, clipped to whichever is smaller; the cursor
/// cell drawn reversed when the program shows it.
pub fn draw_screen(screen: &vt100::Screen, buf: &mut Buffer, area: Rect) {
    let area = area.intersection(buf.area);
    let (rows, cols) = screen.size();
    let cursor = (!screen.hide_cursor()).then(|| screen.cursor_position());
    for row in 0..area.height.min(rows) {
        for col in 0..area.width.min(cols) {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut m = Modifier::empty();
            if cell.bold() {
                m |= Modifier::BOLD;
            }
            if cell.italic() {
                m |= Modifier::ITALIC;
            }
            if cell.underline() {
                m |= Modifier::UNDERLINED;
            }
            if cell.inverse() ^ (cursor == Some((row, col))) {
                m |= Modifier::REVERSED;
            }
            let contents = cell.contents();
            let out = &mut buf[(area.x + col, area.y + row)];
            out.set_symbol(if contents.is_empty() { " " } else { &contents });
            out.set_style(
                Style::default()
                    .fg(color(cell.fgcolor()))
                    .bg(color(cell.bgcolor()))
                    .add_modifier(m),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn screen_cells_map_to_the_buffer() {
        let mut p = vt100::Parser::new(3, 20, 0);
        p.process(b"\x1b[31mred\x1b[0m ok\x1b[1;44mX");
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        draw_screen(p.screen(), &mut buf, area);
        let row: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(row.starts_with("red okX"), "{row:?}");
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(1));
        assert_eq!(buf[(4, 0)].fg, Color::Reset);
        assert_eq!(buf[(6, 0)].bg, Color::Indexed(4));
        // A smaller area than the screen draws what fits and nothing outside it.
        let small = Rect::new(2, 1, 4, 1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        draw_screen(p.screen(), &mut buf, small);
        assert_eq!(buf[(2, 1)].symbol(), "r");
        assert_eq!(buf[(6, 1)].symbol(), " ");
    }

    fn tmux(sock: &str, args: &[&str]) -> bool {
        std::process::Command::new("tmux")
            .arg("-L")
            .arg(sock)
            .args(args)
            .status()
            .is_ok_and(|s| s.success())
    }

    // `set-option -t =name` is "no such session" in tmux: the exact-match form needs `=name:`.
    #[test]
    fn session_options_apply_in_real_tmux() {
        if std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .is_err()
        {
            return;
        }
        let sock = format!("bearclaw-test-opt-{}", std::process::id());
        assert!(tmux(
            &sock,
            &[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "bearclaw-test-opt",
                "cat"
            ]
        ));
        let applied: Vec<bool> = session_options("bearclaw-test-opt")
            .iter()
            .map(|args| tmux(&sock, &args.iter().map(String::as_str).collect::<Vec<_>>()))
            .collect();
        let shown = std::process::Command::new("tmux")
            .args(["-L", &sock, "show-options", "-t", "bearclaw-test-opt"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        kill(&sock);
        assert_eq!(applied, [true, true, true]);
        for want in ["status off", "prefix None", "mouse on"] {
            assert!(shown.contains(want), "{shown}");
        }
    }

    /// Kill the test server and remove its socket file (tmux leaves it behind).
    fn kill(sock: &str) {
        let path = std::process::Command::new("tmux")
            .args(["-L", sock, "display-message", "-p", "#{socket_path}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        tmux(sock, &["kill-server"]);
        if !path.is_empty() {
            let _ = std::fs::remove_file(path);
        }
    }

    fn text(e: &Embedded) -> String {
        e.parser.screen().contents()
    }

    #[test]
    fn embedded_tmux_round_trip() {
        if std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .is_err()
        {
            return;
        }
        let sock = format!("bearclaw-test-{}", std::process::id());
        assert!(tmux(
            &sock,
            &[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "bearclaw-test-embed",
                "-x",
                "80",
                "-y",
                "24",
                "cat"
            ]
        ));
        let mut cmd = portable_pty::CommandBuilder::new("tmux");
        cmd.args(["-L", &sock, "attach-session", "-t", "=bearclaw-test-embed"]);
        let mut e = Embedded::attach_cmd(cmd, 24, 80).unwrap();
        e.write(b"hello\r");
        let until = Instant::now() + Duration::from_secs(5);
        while !text(&e).contains("hello") && Instant::now() < until {
            e.pump();
            std::thread::sleep(Duration::from_millis(20));
        }
        let shown = text(&e);
        kill(&sock);
        assert!(shown.contains("hello"), "{shown:?}");
        e.resize(10, 40);
        let until = Instant::now() + Duration::from_secs(5);
        while e.alive() && Instant::now() < until {
            e.pump();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!e.alive());
    }
}
