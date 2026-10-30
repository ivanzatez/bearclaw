//! 24-bit colour only where the outer terminal draws it. macOS Terminal.app before Tahoe reads
//! `38;2;r;g;b` as separate attributes — a 7 in a colour turns on reverse video, a 41 a red
//! background — for the rest of the frame, even though it sets `COLORTERM=truecolor`. There every
//! RGB colour (the Claw card's druid, a program's own in an embedded terminal) goes out as its
//! nearest xterm-256 colour instead.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

/// Whether the terminal bearclaw runs in draws 24-bit colour.
pub fn truecolor() -> bool {
    truecolor_for(
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var("COLORTERM").ok().as_deref(),
    )
}

fn truecolor_for(program: Option<&str>, colorterm: Option<&str>) -> bool {
    program != Some("Apple_Terminal") && matches!(colorterm, Some("truecolor" | "24bit"))
}

/// The xterm-256 colour nearest to `r g b`: the 6×6×6 cube's or the grey ramp's, whichever is
/// closer (tmux's `colour_find_rgb`).
pub fn to_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [i32; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    let step = |v: u8| match v {
        0..48 => 0,
        48..114 => 1,
        _ => (v as usize - 35) / 40,
    };
    let (qr, qg, qb) = (step(r), step(g), step(b));
    let cube = 16 + 36 * qr as u8 + 6 * qg as u8 + qb as u8;
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let dist = |c: (i32, i32, i32)| (c.0 - r).pow(2) + (c.1 - g).pow(2) + (c.2 - b).pow(2);
    let cube_rgb = (LEVELS[qr], LEVELS[qg], LEVELS[qb]);
    let grey_idx = match (r + g + b) / 3 {
        avg if avg > 238 => 23,
        avg => (avg - 3).max(0) / 10,
    };
    let grey = 8 + 10 * grey_idx;
    if dist((grey, grey, grey)) < dist(cube_rgb) {
        232 + grey_idx as u8
    } else {
        cube
    }
}

/// Every RGB colour in `buf` replaced by its nearest xterm-256 colour.
pub fn downsample(buf: &mut Buffer) {
    let indexed = |c: Color| match c {
        Color::Rgb(r, g, b) => Color::Indexed(to_256(r, g, b)),
        other => other,
    };
    for cell in &mut buf.content {
        cell.fg = indexed(cell.fg);
        cell.bg = indexed(cell.bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn terminal_app_never_gets_24_bit_colour_whatever_colorterm_says() {
        assert!(!truecolor_for(Some("Apple_Terminal"), Some("truecolor")));
        assert!(truecolor_for(Some("iTerm.app"), Some("truecolor")));
        assert!(truecolor_for(Some("WezTerm"), Some("24bit")));
        assert!(!truecolor_for(None, None));
    }

    #[test]
    fn rgb_maps_to_the_nearest_of_the_256() {
        assert_eq!(to_256(0, 0, 0), 16);
        assert_eq!(to_256(255, 255, 255), 231);
        assert_eq!(to_256(0x5f, 0x87, 0xaf), 67);
        assert_eq!(to_256(128, 128, 128), 244);
        assert_eq!(to_256(10, 7, 6), 232);
        assert_eq!(to_256(0xFF, 0x7C, 0x0A), 208);
    }

    #[test]
    fn downsample_replaces_only_rgb() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf[(0, 0)]
            .set_fg(Color::Rgb(255, 255, 255))
            .set_bg(Color::Rgb(0, 0, 0));
        buf[(1, 0)].set_fg(Color::Indexed(3)).set_bg(Color::Reset);
        downsample(&mut buf);
        assert_eq!(
            (buf[(0, 0)].fg, buf[(0, 0)].bg),
            (Color::Indexed(231), Color::Indexed(16))
        );
        assert_eq!(
            (buf[(1, 0)].fg, buf[(1, 0)].bg),
            (Color::Indexed(3), Color::Reset)
        );
        assert_eq!(
            (buf[(2, 0)].fg, buf[(2, 0)].bg),
            (Color::Reset, Color::Reset)
        );
    }
}
