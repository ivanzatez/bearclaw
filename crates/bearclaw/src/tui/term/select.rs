//! Selecting text with the mouse in an embedded terminal.
//!
//! bearclaw captures the mouse (so taps hit tabs and the wheel scrolls), which means
//! the host terminal can't do its own selection. This fills that gap: a left-drag
//! paints a range over the terminal's cell grid, the range is drawn reversed, and on
//! release its text is handed back to copy. A double-click selects the word under it.
//!
//! The range lives in screen-relative cell coordinates — `(row, col)`, 0-based,
//! measured from the top-left of the drawn terminal grid.

use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// Two presses on the same cell closer together than this are a double-click.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// A cell in screen-relative coordinates: `(row, col)`, both 0-based.
pub type Cell = (u16, u16);

/// A selected range of cells, from an anchor to a head, both inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub anchor: Cell,
    pub head: Cell,
}

impl Range {
    /// The range as `(start, end)` in reading order, `start <= end`.
    fn ends(self) -> (Cell, Cell) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Whether the range covers exactly one cell.
    pub fn single(self) -> bool {
        self.anchor == self.head
    }

    /// Whether `(row, col)` falls inside the selection. On an intermediate row the
    /// whole line is selected; the first row is selected from its column onward and
    /// the last row up to its column.
    pub fn contains(self, row: u16, col: u16) -> bool {
        let (s, e) = self.ends();
        if row < s.0 || row > e.0 {
            false
        } else if s.0 == e.0 {
            col >= s.1 && col <= e.1
        } else if row == s.0 {
            col >= s.1
        } else if row == e.0 {
            col <= e.1
        } else {
            true
        }
    }
}

/// The text covered by `range` in `screen`, ready for the clipboard. Trailing blanks
/// on each line are dropped (vt100 doesn't pad them).
pub fn text(range: Range, screen: &vt100::Screen) -> String {
    let (s, e) = range.ends();
    let (_, cols) = screen.size();
    // The head cell is inclusive; `contents_between` takes an exclusive end column.
    let end_col = e.1.saturating_add(1).min(cols);
    screen.contents_between(s.0, s.1, e.0, end_col)
}

/// The word around `(row, col)` in `screen`, or just that cell when it isn't on a
/// word character. Word characters are letters, digits and `_`.
pub fn word_at(screen: &vt100::Screen, row: u16, col: u16) -> Range {
    let (_, cols) = screen.size();
    if cols == 0 || !is_word(cell_char(screen, row, col)) {
        return Range {
            anchor: (row, col),
            head: (row, col),
        };
    }
    let mut start = col;
    while start > 0 && is_word(cell_char(screen, row, start - 1)) {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < cols && is_word(cell_char(screen, row, end + 1)) {
        end += 1;
    }
    Range {
        anchor: (row, start),
        head: (row, end),
    }
}

fn is_word(c: Option<char>) -> bool {
    matches!(c, Some(c) if c.is_alphanumeric() || c == '_')
}

fn cell_char(screen: &vt100::Screen, row: u16, col: u16) -> Option<char> {
    screen
        .cell(row, col)
        .and_then(|c| c.contents().chars().next())
}

/// The mouse selection state for one embedded terminal.
#[derive(Debug, Default)]
pub struct Selection {
    range: Option<Range>,
    dragging: bool,
    /// The head moved away from the anchor at least once this drag.
    moved: bool,
    /// The current range came from a double-click, so a release copies it even
    /// without a drag.
    word: bool,
    last_click: Option<(Instant, Cell)>,
}

impl Selection {
    /// The range to draw, if any.
    pub fn range(&self) -> Option<Range> {
        self.range
    }

    /// Whether a button is down and a selection is being dragged.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Drop any selection and any drag in progress (but remember the last click, so a
    /// double-click still works after the highlight is cleared by typing).
    pub fn clear(&mut self) {
        self.range = None;
        self.dragging = false;
        self.moved = false;
        self.word = false;
    }

    /// A left-button press at `cell`. A second press on the same cell within
    /// [`DOUBLE_CLICK`] selects the word there.
    pub fn press(&mut self, cell: Cell, now: Instant, screen: &vt100::Screen) {
        let double = matches!(
            self.last_click,
            Some((t, c)) if c == cell && now.saturating_duration_since(t) <= DOUBLE_CLICK
        );
        self.last_click = Some((now, cell));
        self.dragging = true;
        self.moved = false;
        if double {
            self.range = Some(word_at(screen, cell.0, cell.1));
            self.word = true;
        } else {
            self.range = Some(Range {
                anchor: cell,
                head: cell,
            });
            self.word = false;
        }
    }

    /// The mouse moved to `cell` with the button held; extends the selection.
    pub fn drag(&mut self, cell: Cell) {
        if !self.dragging {
            return;
        }
        if let Some(r) = self.range.as_mut() {
            r.head = cell;
            if cell != r.anchor {
                self.moved = true;
            }
        }
    }

    /// The button came up. Returns the range to copy when the gesture selected
    /// something (a drag, or a double-click word); a plain click selects nothing and
    /// clears the range instead.
    pub fn release(&mut self) -> Option<Range> {
        let was = self.dragging;
        self.dragging = false;
        if was && (self.moved || self.word) {
            self.range
        } else {
            self.range = None;
            self.word = false;
            None
        }
    }
}

/// What a left-button mouse event means for selection inside a terminal drawn at
/// `screen`. Presses outside the grid, and non-left buttons, are [`SelAct::Ignore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelAct {
    Ignore,
    /// A press began a selection at this (screen-relative) cell.
    Press(Cell),
    /// A drag extended the selection to this (clamped, screen-relative) cell.
    Drag(Cell),
    /// The button was released.
    Release,
}

pub fn sel_action(m: &MouseEvent, screen: Rect) -> SelAct {
    let inside = m.row >= screen.y
        && m.row < screen.y.saturating_add(screen.height)
        && m.column >= screen.x
        && m.column < screen.x.saturating_add(screen.width);
    // Screen-relative cell, clamped to the grid so a drag past its edge still extends.
    let rel = || -> Cell {
        let row = m
            .row
            .saturating_sub(screen.y)
            .min(screen.height.saturating_sub(1));
        let col = m
            .column
            .saturating_sub(screen.x)
            .min(screen.width.saturating_sub(1));
        (row, col)
    };
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if inside => SelAct::Press(rel()),
        MouseEventKind::Drag(MouseButton::Left) => SelAct::Drag(rel()),
        MouseEventKind::Up(MouseButton::Left) => SelAct::Release,
        _ => SelAct::Ignore,
    }
}

/// Paint `range` over `area` of `buf` by reversing the selected cells.
pub fn highlight(buf: &mut Buffer, area: Rect, range: Range) {
    for row in 0..area.height {
        for col in 0..area.width {
            if range.contains(row, col) {
                buf[(area.x + col, area.y + row)].modifier ^= Modifier::REVERSED;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn parser(rows: u16, cols: u16, feed: &str) -> vt100::Parser {
        let mut p = vt100::Parser::new(rows, cols, 0);
        p.process(feed.as_bytes());
        p
    }

    fn ev(kind: MouseEventKind, row: u16, column: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn range_contains_on_a_single_row() {
        let r = Range {
            anchor: (1, 2),
            head: (1, 5),
        };
        assert!(r.contains(1, 2));
        assert!(r.contains(1, 5));
        assert!(!r.contains(1, 1));
        assert!(!r.contains(1, 6));
        assert!(!r.contains(0, 3));
        assert!(!r.contains(2, 3));
    }

    #[test]
    fn range_contains_spanning_rows_selects_whole_middle_lines() {
        let r = Range {
            anchor: (1, 3),
            head: (3, 2),
        };
        // First row: from the anchor column to the end of the line.
        assert!(r.contains(1, 3));
        assert!(r.contains(1, 999));
        assert!(!r.contains(1, 2));
        // Middle row: the whole line.
        assert!(r.contains(2, 0));
        assert!(r.contains(2, 999));
        // Last row: up to and including the head column.
        assert!(r.contains(3, 0));
        assert!(r.contains(3, 2));
        assert!(!r.contains(3, 3));
        // Outside the row span.
        assert!(!r.contains(0, 3));
        assert!(!r.contains(4, 0));
    }

    #[test]
    fn range_contains_normalises_a_backwards_selection() {
        let r = Range {
            anchor: (3, 2),
            head: (1, 3),
        };
        assert!(r.contains(2, 0));
        assert!(r.contains(1, 3));
        assert!(!r.contains(1, 2));
    }

    #[test]
    fn range_single_detects_one_cell() {
        assert!(
            Range {
                anchor: (2, 2),
                head: (2, 2)
            }
            .single()
        );
        assert!(
            !Range {
                anchor: (2, 2),
                head: (2, 3)
            }
            .single()
        );
    }

    #[test]
    fn text_reads_a_run_on_one_row() {
        let p = parser(24, 80, "hello world");
        assert_eq!(
            text(
                Range {
                    anchor: (0, 0),
                    head: (0, 4)
                },
                p.screen()
            ),
            "hello"
        );
        assert_eq!(
            text(
                Range {
                    anchor: (0, 0),
                    head: (0, 0)
                },
                p.screen()
            ),
            "h"
        );
    }

    #[test]
    fn text_spans_rows_with_a_newline() {
        let p = parser(24, 80, "ab\r\ncd");
        assert_eq!(
            text(
                Range {
                    anchor: (0, 1),
                    head: (1, 0)
                },
                p.screen()
            ),
            "b\nc"
        );
    }

    #[test]
    fn text_drops_trailing_blanks_and_normalises_direction() {
        let p = parser(24, 80, "hi");
        // Selecting past the text includes no trailing spaces.
        assert_eq!(
            text(
                Range {
                    anchor: (0, 0),
                    head: (0, 6)
                },
                p.screen()
            ),
            "hi"
        );
        // A backwards selection reads the same text.
        assert_eq!(
            text(
                Range {
                    anchor: (0, 4),
                    head: (0, 0)
                },
                p.screen()
            ),
            "hi"
        );
    }

    #[test]
    fn word_at_selects_the_word_under_the_cell() {
        let p = parser(24, 80, "foo_bar baz");
        assert_eq!(
            word_at(p.screen(), 0, 1),
            Range {
                anchor: (0, 0),
                head: (0, 6)
            }
        );
        assert_eq!(
            word_at(p.screen(), 0, 9),
            Range {
                anchor: (0, 8),
                head: (0, 10)
            }
        );
    }

    #[test]
    fn word_at_on_whitespace_is_a_single_cell() {
        let p = parser(24, 80, "foo bar");
        assert_eq!(
            word_at(p.screen(), 0, 3),
            Range {
                anchor: (0, 3),
                head: (0, 3)
            }
        );
    }

    #[test]
    fn a_plain_click_selects_nothing() {
        let p = parser(24, 80, "hello");
        let mut s = Selection::default();
        s.press((0, 1), Instant::now(), p.screen());
        assert_eq!(
            s.range(),
            Some(Range {
                anchor: (0, 1),
                head: (0, 1)
            })
        );
        assert!(s.is_dragging());
        assert_eq!(s.release(), None);
        assert_eq!(s.range(), None);
    }

    #[test]
    fn a_drag_selects_and_keeps_the_highlight_after_release() {
        let p = parser(24, 80, "hello world");
        let mut s = Selection::default();
        s.press((0, 0), Instant::now(), p.screen());
        s.drag((0, 4));
        let want = Range {
            anchor: (0, 0),
            head: (0, 4),
        };
        assert_eq!(s.range(), Some(want));
        assert_eq!(s.release(), Some(want));
        // The highlight stays so the user sees what was copied.
        assert_eq!(s.range(), Some(want));
    }

    #[test]
    fn a_drag_back_to_the_anchor_selects_nothing() {
        let p = parser(24, 80, "hello");
        let mut s = Selection::default();
        s.press((0, 2), Instant::now(), p.screen());
        s.drag((0, 2));
        assert_eq!(s.release(), None);
    }

    #[test]
    fn a_double_click_selects_the_word() {
        let p = parser(24, 80, "foo_bar baz");
        let mut s = Selection::default();
        let t0 = Instant::now();
        s.press((0, 1), t0, p.screen());
        assert_eq!(s.release(), None);
        s.press((0, 1), t0 + Duration::from_millis(100), p.screen());
        let word = Range {
            anchor: (0, 0),
            head: (0, 6),
        };
        assert_eq!(s.range(), Some(word));
        assert_eq!(s.release(), Some(word));
    }

    #[test]
    fn two_slow_clicks_are_not_a_double_click() {
        let p = parser(24, 80, "foo_bar baz");
        let mut s = Selection::default();
        let t0 = Instant::now();
        s.press((0, 1), t0, p.screen());
        assert_eq!(s.release(), None);
        s.press((0, 1), t0 + Duration::from_millis(600), p.screen());
        assert_eq!(
            s.range(),
            Some(Range {
                anchor: (0, 1),
                head: (0, 1)
            })
        );
    }

    #[test]
    fn clear_drops_the_selection() {
        let p = parser(24, 80, "hello");
        let mut s = Selection::default();
        s.press((0, 0), Instant::now(), p.screen());
        s.drag((0, 3));
        s.clear();
        assert_eq!(s.range(), None);
        assert!(!s.is_dragging());
    }

    #[test]
    fn sel_action_starts_a_selection_inside_the_grid() {
        // Grid drawn at row 2, full width; height 22.
        let screen = Rect::new(0, 2, 80, 22);
        assert_eq!(
            sel_action(&ev(MouseEventKind::Down(MouseButton::Left), 5, 10), screen),
            SelAct::Press((3, 10))
        );
        assert_eq!(
            sel_action(&ev(MouseEventKind::Down(MouseButton::Left), 2, 0), screen),
            SelAct::Press((0, 0))
        );
    }

    #[test]
    fn sel_action_ignores_presses_above_the_grid() {
        let screen = Rect::new(0, 2, 80, 22);
        // Row 1 is the header, not the grid.
        assert_eq!(
            sel_action(&ev(MouseEventKind::Down(MouseButton::Left), 1, 10), screen),
            SelAct::Ignore
        );
    }

    #[test]
    fn sel_action_clamps_a_drag_that_leaves_the_grid() {
        let screen = Rect::new(0, 2, 80, 22);
        // Below and to the right: clamp to the bottom-right cell (21, 79).
        assert_eq!(
            sel_action(
                &ev(MouseEventKind::Drag(MouseButton::Left), 100, 200),
                screen
            ),
            SelAct::Drag((21, 79))
        );
        // Above the grid: clamp to the top.
        assert_eq!(
            sel_action(&ev(MouseEventKind::Drag(MouseButton::Left), 0, 5), screen),
            SelAct::Drag((0, 5))
        );
    }

    #[test]
    fn sel_action_release_and_other_buttons() {
        let screen = Rect::new(0, 2, 80, 22);
        assert_eq!(
            sel_action(&ev(MouseEventKind::Up(MouseButton::Left), 5, 10), screen),
            SelAct::Release
        );
        assert_eq!(
            sel_action(&ev(MouseEventKind::Down(MouseButton::Right), 5, 10), screen),
            SelAct::Ignore
        );
        assert_eq!(
            sel_action(&ev(MouseEventKind::ScrollUp, 5, 10), screen),
            SelAct::Ignore
        );
    }

    #[test]
    fn highlight_reverses_only_the_selected_cells() {
        let area = Rect::new(0, 0, 6, 2);
        let mut buf = Buffer::empty(area);
        let r = Range {
            anchor: (0, 1),
            head: (0, 3),
        };
        highlight(&mut buf, area, r);
        assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(1, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(2, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(3, 0)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(4, 0)].modifier.contains(Modifier::REVERSED));
        // A different row is untouched.
        assert!(!buf[(2, 1)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn highlight_maps_through_an_offset_area() {
        let area = Rect::new(3, 2, 5, 3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 6));
        let r = Range {
            anchor: (1, 0),
            head: (1, 1),
        };
        highlight(&mut buf, area, r);
        // Row 1, cols 0..=1 of the area map to buffer (3,3) and (4,3).
        assert!(buf[(3, 3)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(4, 3)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(5, 3)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(3, 2)].modifier.contains(Modifier::REVERSED));
    }
}
