//! Crossterm key events back into the bytes a terminal would have sent.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn modifier(m: KeyModifiers) -> u8 {
    1 + u8::from(m.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(m.contains(KeyModifiers::ALT))
        + 4 * u8::from(m.contains(KeyModifiers::CONTROL))
}

/// `CSI <final>`, or `CSI 1;<mod> <final>` with modifiers.
fn csi(m: KeyModifiers, last: char) -> Vec<u8> {
    match modifier(m) {
        1 => format!("\x1b[{last}"),
        n => format!("\x1b[1;{n}{last}"),
    }
    .into_bytes()
}

/// `CSI <n> ~`, or `CSI <n>;<mod> ~`.
fn tilde(m: KeyModifiers, n: u8) -> Vec<u8> {
    match modifier(m) {
        1 => format!("\x1b[{n}~"),
        k => format!("\x1b[{n};{k}~"),
    }
    .into_bytes()
}

/// The bytes for a key press, as xterm sends them. `None` for keys a terminal has no
/// bytes for.
pub fn encode(k: KeyEvent) -> Option<Vec<u8>> {
    let m = k.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);
    let shift = m.contains(KeyModifiers::SHIFT);
    Some(match k.code {
        KeyCode::Char(c) if ctrl => {
            let b = match c {
                ' ' | '@' | '2' => 0x00,
                'a'..='z' => c as u8 - b'a' + 1,
                'A'..='Z' => c as u8 - b'A' + 1,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '7' | '/' => 0x1f,
                _ => return None,
            };
            if alt { vec![0x1b, b] } else { vec![b] }
        }
        KeyCode::Char(c) => {
            let mut out = if alt { vec![0x1b] } else { Vec::new() };
            out.extend(c.to_string().as_bytes());
            out
        }
        // Claude Code reads ESC-CR as "newline, don't send".
        KeyCode::Enter if shift || alt => b"\x1b\r".to_vec(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace if alt => b"\x1b\x7f".to_vec(),
        KeyCode::Backspace => b"\x7f".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Up => csi(m, 'A'),
        KeyCode::Down => csi(m, 'B'),
        KeyCode::Right => csi(m, 'C'),
        KeyCode::Left => csi(m, 'D'),
        KeyCode::Home => csi(m, 'H'),
        KeyCode::End => csi(m, 'F'),
        KeyCode::Insert => tilde(m, 2),
        KeyCode::Delete => tilde(m, 3),
        KeyCode::PageUp => tilde(m, 5),
        KeyCode::PageDown => tilde(m, 6),
        KeyCode::F(n @ 1..=4) => vec![0x1b, b'O', b'P' + (n - 1)],
        KeyCode::F(n @ 5..=12) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde(m, CODES[(n - 5) as usize])
        }
        _ => return None,
    })
}

/// Pasted text, bracketed so the app knows it was pasted, not typed.
pub fn paste(text: &str) -> Vec<u8> {
    format!("\x1b[200~{text}\x1b[201~").into_bytes()
}

/// A mouse wheel notch at a 0-based cell, as an SGR mouse report.
pub fn wheel(up: bool, col: u16, row: u16) -> Vec<u8> {
    let button = if up { 64 } else { 65 };
    format!("\x1b[<{button};{};{}M", col + 1, row + 1).into_bytes()
}

/// Ctrl+L (or Ctrl+], which some keyboards can't type) — the keys the embedded terminal keeps
/// for itself: back to the dashboard. The program inside never sees them.
pub fn is_leave(k: KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('l' | ']' | '5'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode as C, KeyEvent, KeyModifiers as M};

    fn e(c: C, m: M) -> Option<Vec<u8>> {
        encode(KeyEvent::new(c, m))
    }
    fn b(s: &str) -> Option<Vec<u8>> {
        Some(s.as_bytes().to_vec())
    }

    #[test]
    fn keys_encode_like_xterm() {
        assert_eq!(e(C::Char('a'), M::NONE), b("a"));
        assert_eq!(e(C::Char('A'), M::SHIFT), b("A"));
        assert_eq!(e(C::Char('č'), M::NONE), b("č"));
        assert_eq!(e(C::Enter, M::NONE), b("\r"));
        assert_eq!(e(C::Enter, M::SHIFT), b("\x1b\r"));
        assert_eq!(e(C::Enter, M::ALT), b("\x1b\r"));
        assert_eq!(e(C::Tab, M::NONE), b("\t"));
        assert_eq!(e(C::BackTab, M::SHIFT), b("\x1b[Z"));
        assert_eq!(e(C::Backspace, M::NONE), b("\x7f"));
        assert_eq!(e(C::Esc, M::NONE), b("\x1b"));
        assert_eq!(e(C::Up, M::NONE), b("\x1b[A"));
        assert_eq!(e(C::Down, M::NONE), b("\x1b[B"));
        assert_eq!(e(C::Right, M::NONE), b("\x1b[C"));
        assert_eq!(e(C::Left, M::NONE), b("\x1b[D"));
        assert_eq!(e(C::Up, M::SHIFT), b("\x1b[1;2A"));
        assert_eq!(e(C::Left, M::ALT), b("\x1b[1;3D"));
        assert_eq!(e(C::Right, M::CONTROL), b("\x1b[1;5C"));
        assert_eq!(e(C::Home, M::NONE), b("\x1b[H"));
        assert_eq!(e(C::End, M::NONE), b("\x1b[F"));
        assert_eq!(e(C::PageUp, M::NONE), b("\x1b[5~"));
        assert_eq!(e(C::PageDown, M::NONE), b("\x1b[6~"));
        assert_eq!(e(C::Delete, M::NONE), b("\x1b[3~"));
        assert_eq!(e(C::Insert, M::NONE), b("\x1b[2~"));
        assert_eq!(e(C::F(1), M::NONE), b("\x1bOP"));
        assert_eq!(e(C::F(4), M::NONE), b("\x1bOS"));
        assert_eq!(e(C::F(5), M::NONE), b("\x1b[15~"));
        assert_eq!(e(C::Char('a'), M::CONTROL), Some(vec![0x01]));
        assert_eq!(e(C::Char('c'), M::CONTROL), Some(vec![0x03]));
        assert_eq!(e(C::Char('z'), M::CONTROL), Some(vec![0x1a]));
        assert_eq!(e(C::Char(' '), M::CONTROL), Some(vec![0x00]));
        assert_eq!(e(C::Char('b'), M::ALT), b("\x1bb"));
        assert_eq!(paste("hi"), b"\x1b[200~hi\x1b[201~".to_vec());
        assert_eq!(wheel(true, 0, 0), b"\x1b[<64;1;1M".to_vec());
        assert_eq!(wheel(false, 4, 9), b"\x1b[<65;5;10M".to_vec());
    }

    #[test]
    fn leave_is_ctrl_l_or_ctrl_right_bracket() {
        assert!(is_leave(KeyEvent::new(C::Char('l'), M::CONTROL)));
        assert!(!is_leave(KeyEvent::new(C::Char('l'), M::NONE)));
        assert!(is_leave(KeyEvent::new(C::Char(']'), M::CONTROL)));
        // 0x1d arrives as Ctrl-5 in crossterm's legacy decoding.
        assert!(is_leave(KeyEvent::new(C::Char('5'), M::CONTROL)));
        assert!(!is_leave(KeyEvent::new(C::Esc, M::NONE)));
        assert!(!is_leave(KeyEvent::new(C::Char(']'), M::NONE)));
    }
}
