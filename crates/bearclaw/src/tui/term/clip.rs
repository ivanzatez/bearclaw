//! Copying selected terminal text to the clipboard.
//!
//! On the machine you're sitting at, the local clipboard tool (`pbcopy`, else
//! `wl-copy` / `xclip`). When bearclaw runs on the far end of ssh or mosh, an
//! OSC 52 escape written to the outer terminal instead, so the text lands on the
//! clipboard of the machine in front of you rather than the remote one.

use std::io::{self, Write};

/// Where a copy was (or should be) sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sink {
    /// A local clipboard tool: `pbcopy` / `wl-copy` / `xclip`.
    Local,
    /// An OSC 52 escape written to the outer terminal (ssh / mosh).
    Osc52,
}

impl Sink {
    pub fn label(self) -> &'static str {
        match self {
            Sink::Local => "clipboard",
            Sink::Osc52 => "osc52",
        }
    }
}

/// Choose the sink from the environment. OSC 52 when a session variable says we're
/// on the far end of ssh (or mosh, which keeps the ssh variables its bootstrap set);
/// the local clipboard otherwise. `get` reads an environment variable.
pub fn sink(get: impl Fn(&str) -> Option<String>) -> Sink {
    let set = |k: &str| get(k).is_some_and(|v| !v.is_empty());
    if set("SSH_CONNECTION") || set("SSH_TTY") || set("SSH_CLIENT") {
        Sink::Osc52
    } else {
        Sink::Local
    }
}

/// Standard base64 of `bytes`, no line breaks.
pub fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b1 = u32::from(chunk[0]);
        let b2 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b3 = u32::from(*chunk.get(2).unwrap_or(&0));
        let n = (b1 << 16) | (b2 << 8) | b3;
        out.push(A[(n >> 18 & 63) as usize] as char);
        out.push(A[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// The OSC 52 escape that sets the system clipboard to `text`. When bearclaw is
/// itself running inside a tmux (`in_tmux`), it is wrapped in tmux's passthrough so
/// the escape reaches the real terminal rather than being swallowed by tmux.
pub fn osc52(text: &str, in_tmux: bool) -> Vec<u8> {
    let inner = format!("\x1b]52;c;{}\x07", b64(text.as_bytes()));
    if in_tmux {
        // tmux DCS passthrough: `ESC P tmux ; <payload, ESCs doubled> ESC \`.
        format!("\x1bPtmux;{}\x1b\\", inner.replace('\x1b', "\x1b\x1b")).into_bytes()
    } else {
        inner.into_bytes()
    }
}

/// Put `text` on the clipboard, choosing the sink from the environment; returns
/// which sink was used.
pub fn copy(text: &str) -> Result<Sink, String> {
    let in_tmux = env_set("TMUX");
    match sink(|k| std::env::var(k).ok()) {
        Sink::Osc52 => {
            write_osc52(text, in_tmux)?;
            Ok(Sink::Osc52)
        }
        Sink::Local => match local(text) {
            Ok(()) => Ok(Sink::Local),
            // No local tool here — a capable terminal may still take the escape.
            Err(_) => {
                write_osc52(text, in_tmux)?;
                Ok(Sink::Osc52)
            }
        },
    }
}

fn env_set(key: &str) -> bool {
    std::env::var(key).is_ok_and(|v| !v.is_empty())
}

fn write_osc52(text: &str, in_tmux: bool) -> Result<(), String> {
    let mut out = io::stdout();
    out.write_all(&osc52(text, in_tmux))
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())
}

/// Put `text` on a local clipboard (`pbcopy`, else `wl-copy` / `xclip`).
pub fn local(text: &str) -> Result<(), String> {
    let tools: [(&str, &[&str]); 3] = [
        ("pbcopy", &[]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
    ];
    for (bin, args) in tools {
        let Ok(mut child) = std::process::Command::new(bin)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        return child
            .wait()
            .map_err(|e| format!("{bin}: {e}"))
            .and_then(|st| st.success().then_some(()).ok_or(format!("{bin} failed")));
    }
    Err("no clipboard tool (pbcopy, wl-copy, xclip)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_vectors() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foob"), "Zm9vYg==");
        assert_eq!(b64(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
        // Non-ASCII bytes round-trip through the alphabet too.
        assert_eq!(b64("č".as_bytes()), "xI0=");
    }

    #[test]
    fn osc52_is_the_bare_escape_when_not_in_tmux() {
        assert_eq!(osc52("hi", false), b"\x1b]52;c;aGk=\x07".to_vec());
    }

    #[test]
    fn osc52_is_wrapped_for_an_outer_tmux_with_every_esc_doubled() {
        // tmux passthrough: DCS `tmux;` … ST, inner ESCs doubled.
        assert_eq!(
            osc52("hi", true),
            b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\".to_vec()
        );
    }

    #[test]
    fn ssh_variables_pick_osc52_both_for_ssh_and_mosh() {
        let only = |key: &'static str| move |k: &str| (k == key).then(|| "x".to_string());
        assert_eq!(sink(only("SSH_CONNECTION")), Sink::Osc52);
        assert_eq!(sink(only("SSH_TTY")), Sink::Osc52);
        assert_eq!(sink(only("SSH_CLIENT")), Sink::Osc52);
    }

    #[test]
    fn no_ssh_variable_picks_the_local_clipboard() {
        assert_eq!(sink(|_| None), Sink::Local);
        // An empty variable counts as unset.
        assert_eq!(sink(|k| (k == "SSH_TTY").then(String::new)), Sink::Local);
    }

    #[test]
    fn sink_labels_are_distinct() {
        assert_eq!(Sink::Local.label(), "clipboard");
        assert_eq!(Sink::Osc52.label(), "osc52");
    }
}
