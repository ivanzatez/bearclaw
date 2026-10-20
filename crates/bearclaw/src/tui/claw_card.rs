//! The Claw tab's neofetch-style character card: the druid claw sprite in half-blocks over a
//! few live stats. A left column beside Claw's terminal; the loop hides it on a narrow pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::core::discovery::Session;
use crate::core::{brain, todo};
use crate::tui::claw_sprite::{H, PALETTE, PIXELS, W};
use crate::tui::tabs::CardStat;

/// Columns the card column takes: the 52px sprite plus a one-column gutter each side.
pub const WIDTH: u16 = W as u16 + 2;
/// Below this body width the card is dropped and the terminal gets the whole pane.
pub const MIN_WIDTH: u16 = 140;

const ORANGE: Color = Color::Rgb(0xFF, 0x7C, 0x0A); // druid orange, the label colour
const FG: Color = Color::Rgb(222, 214, 200);
const DIM: Color = Color::Rgb(120, 112, 104);

/// The druid palette, as a swatch strip under the stats (the generator's `sw`, orange last).
const SWATCHES: [(u8, u8, u8); 9] = [
    (10, 7, 6),
    (66, 41, 24),
    (104, 66, 38),
    (150, 102, 60),
    (200, 150, 96),
    (120, 116, 112),
    (190, 184, 172),
    (246, 240, 226),
    (255, 124, 10),
];

/// A snapshot of what the card shows, gathered off the dashboard's own state plus the few
/// numbers the PRs/Usage tabs cache (`stat`). Cheap enough to rebuild on a timer.
#[derive(Clone)]
pub struct Stats {
    pub user: String,
    pub sessions: usize,
    pub busy: usize,
    pub idle: usize,
    pub waiting: usize,
    pub todo_doing: usize,
    pub brain_notes: usize,
    pub stat: CardStat,
}

impl Stats {
    pub fn gather(rows: &[Session], stat: CardStat) -> Stats {
        let busy = rows.iter().filter(|r| r.status == "busy").count();
        let idle = rows.iter().filter(|r| r.status == "idle").count();
        let waiting = rows.iter().filter(|r| r.is_waiting()).count();
        let todo_doing = todo::load(&todo::path())
            .map(|t| {
                t.items
                    .iter()
                    .filter(|i| i.status == todo::Status::Doing)
                    .count()
            })
            .unwrap_or(0);
        let brain_notes = brain::list(&brain::root()).len();
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        Stats {
            user,
            sessions: rows.len(),
            busy,
            idle,
            waiting,
            todo_doing,
            brain_notes,
            stat,
        }
    }
}

/// `label: value`, the label in bold druid orange and the value in the foreground grey.
fn info(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label}: "),
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(value, Style::default().fg(FG)),
    ])
}

/// The sprite as half-block rows: one cell = one pixel column, one row = two pixels (`▀` =
/// upper pixel in the foreground, lower in the background).
fn sprite_lines() -> Vec<Line<'static>> {
    let at = |x: usize, y: usize| {
        let (r, g, b) = PALETTE[PIXELS[y * W + x] as usize];
        Color::Rgb(r, g, b)
    };
    let mut out = Vec::with_capacity(H / 2);
    let mut y = 0;
    while y + 1 < H {
        let spans = (0..W)
            .map(|x| Span::styled("▀", Style::default().fg(at(x, y)).bg(at(x, y + 1))))
            .collect::<Vec<_>>();
        out.push(Line::from(spans));
        y += 2;
    }
    out
}

/// The druid palette strip: each colour a solid six-column block.
fn palette_bar() -> Line<'static> {
    Line::from(
        SWATCHES
            .iter()
            .map(|&(r, g, b)| Span::styled("      ", Style::default().bg(Color::Rgb(r, g, b))))
            .collect::<Vec<_>>(),
    )
}

/// Draw the card into `area` (one column of gutter on the left).
pub fn render(f: &mut Frame, area: Rect, s: &Stats) {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let sessions = plural(s.waiting, "session", "sessions");
    let waiting = match s.stat.pr_waiting {
        Some(pr) => format!("{sessions} · {}", plural(pr, "PR", "PRs")),
        None => sessions,
    };
    let usage = match (s.stat.usage_session, s.stat.usage_week) {
        (Some(a), Some(b)) => format!("{a:.0}% session · {b:.0}% week"),
        (Some(a), None) => format!("{a:.0}% session"),
        _ => "…".into(),
    };

    let mut lines = sprite_lines();
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        format!("{}@bearclaw", s.user),
        Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "─".repeat(13),
        Style::default().fg(DIM),
    )));
    lines.push(info("Class", "Druid · Feral".into()));
    lines.push(info(
        "Sessions",
        format!("{} ({} busy, {} idle)", s.sessions, s.busy, s.idle),
    ));
    lines.push(info("Waiting on you", waiting));
    lines.push(info("Todo", format!("{} in progress", s.todo_doing)));
    lines.push(info("Usage", usage));
    lines.push(info("Brain", format!("{} notes", s.brain_notes)));
    lines.push(Line::default());
    lines.push(palette_bar());

    let inner = Rect {
        x: area.x + 1,
        y: area.y,
        width: area.width.saturating_sub(1),
        height: area.height,
    };
    f.render_widget(Paragraph::new(lines), inner);
}
