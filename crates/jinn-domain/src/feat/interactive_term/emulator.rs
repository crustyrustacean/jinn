//! Terminal emulation — bytes in, rendered screen out.
//!
//! Wraps a [`vt100::Parser`] (the in-process terminal emulator). Raw pty
//! output is fed to [`Emulator::feed`]; the parsed screen is read back as
//! plain text (for tool results) or styled cells (for the takeover view).
//!
//! The scrollback transcript is jinn's own ring, not vt100's scrollback: the
//! transcript must capture observed screen states (for the kill result's
//! tail), which [`Emulator::sync_transcript`] appends at settle time. Capping
//! the ring keeps memory bounded for chatty programs.

use std::collections::VecDeque;

use vt100::Parser;

/// Smallest emulator grid vt100 0.15 can hold without panicking. Audit of
/// every `u16` subtraction in its grid: unguarded `rows - 1` (in `Grid::new`,
/// `clear`, `set_size`, `set_scroll_region`) needs rows >= 2, and
/// `col_wrap`'s `prev_pos.row -= scrolled` underflows only on a 1-row grid
/// (a scroll implies the cursor sat at `scroll_bottom`, which a scroll region
/// keeps >= 1). All remaining sites are guarded or need cols >= width <= 2.
const EMULATOR_MIN_ROWS: u16 = 2;
/// See [`EMULATOR_MIN_ROWS`]: unguarded `cols - width` with wide (2-cell)
/// writes needs cols >= 2.
const EMULATOR_MIN_COLS: u16 = 2;

/// How many scrollback rows vt100's grid retains.
const EMULATOR_SCROLLBACK_ROWS: usize = 1000;

/// Default transcript line cap; the kill result reports only the tail.
const DEFAULT_TRANSCRIPT_LINES: usize = 500;

/// In-process terminal emulator over a byte stream.
///
/// Owns the parser and the append-only transcript. All methods take `&mut
/// self` for feeding but `&self` for reads, so a snapshot can be taken from a
/// shared handle.
pub struct Emulator {
    parser: Parser,
    transcript: VecDeque<String>,
    transcript_cap: usize,
}

impl Default for Emulator {
    fn default() -> Self {
        Self::new(24, 80, DEFAULT_TRANSCRIPT_LINES)
    }
}

impl Emulator {
    /// Creates an emulator of `rows`×`cols` cells with a transcript capped at
    /// `transcript_cap` lines.
    ///
    /// Dimensions are clamped to a 2×2 floor: vt100's grid arithmetic
    /// (`rows - 1` in `Grid::new`, `set_size`, `scroll_up`,
    /// `set_scroll_region`) panics on a 0-row terminal, and its wrap logic
    /// (`col_wrap`'s `prev_pos.row -= scrolled`) underflows on any 1-row
    /// grid whose content wraps. A zeroed size from upstream must never
    /// reach it.
    #[must_use]
    pub fn new(rows: u16, cols: u16, transcript_cap: usize) -> Self {
        Self {
            parser: Parser::new(
                rows.max(EMULATOR_MIN_ROWS),
                cols.max(EMULATOR_MIN_COLS),
                EMULATOR_SCROLLBACK_ROWS,
            ),
            transcript: VecDeque::with_capacity(transcript_cap.min(64)),
            transcript_cap: transcript_cap.max(1),
        }
    }

    /// Feeds raw pty output into the emulator, updating screen and transcript.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Snapshots the visible screen into the transcript ring.
    ///
    /// Called at settle time: each settle appends the latest screen state,
    /// so the transcript reads as a sequence of observed screens. Non-empty
    /// trailing duplicate screens (repaints with no change) are skipped.
    pub fn sync_transcript(&mut self) {
        let text = self.parser.screen().contents();
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            return;
        }
        if self.transcript.back().is_some_and(|last| last == trimmed) {
            return;
        }
        if self.transcript.len() >= self.transcript_cap {
            self.transcript.pop_front();
        }
        self.transcript.push_back(trimmed.to_owned());
    }

    /// The rendered screen as plain text with trailing blank columns/rows
    /// stripped per line and overall trailing emptiness trimmed.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let (rows, cols) = self.parser.screen().size();
        let screen = self.parser.screen();
        let mut lines = Vec::with_capacity(usize::from(rows));
        for row in screen.rows(0, cols) {
            lines.push(row.trim_end().to_owned());
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// The styled cells of the visible screen for the takeover renderer.
    ///
    /// Returns `(rows, cols, cells)` where cells are laid out row-major.
    /// Wide characters occupy their leading cell; their continuation cell is
    /// [`TermCell::WideSpacer`].
    #[must_use]
    pub fn cells(&self) -> ScreenCells {
        let (rows, cols) = self.parser.screen().size();
        let screen = self.parser.screen();
        let mut cells = Vec::with_capacity(usize::from(rows) * usize::from(cols));
        for row in 0..rows {
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    cells.push(TermCell::Blank);
                    continue;
                };
                if cell.is_wide_continuation() {
                    cells.push(TermCell::WideSpacer);
                    continue;
                }
                let style = CellStyle {
                    fg: cell.fgcolor().into(),
                    bg: cell.bgcolor().into(),
                    bold: cell.bold(),
                    italic: cell.italic(),
                    underline: cell.underline(),
                    inverse: cell.inverse(),
                };
                if !cell.has_contents() {
                    cells.push(TermCell::Styled { ch: ' ', style });
                    continue;
                }
                let text = cell.contents();
                // A wide cell's `contents()` yields the full grapheme; render
                // its first char (ratatui handles the width when drawing).
                let ch = text.chars().next().unwrap_or(' ');
                cells.push(TermCell::Styled { ch, style });
            }
        }
        ScreenCells { rows, cols, cells }
    }

    /// The cursor position as `(row, col)`.
    #[must_use]
    pub fn cursor_position(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    /// Whether the program hid the cursor (fullscreen apps during repaints).
    #[must_use]
    pub fn cursor_hidden(&self) -> bool {
        self.parser.screen().hide_cursor()
    }

    /// The emulator size as `(rows, cols)`.
    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    /// Resizes the emulator grid. Must mirror the pty resize.
    ///
    /// Dimensions are clamped to a 2×2 floor — same vt100 panics as
    /// [`Emulator::new`].
    pub fn set_size(&mut self, rows: u16, cols: u16) {
        self.parser
            .set_size(rows.max(EMULATOR_MIN_ROWS), cols.max(EMULATOR_MIN_COLS));
    }

    /// The transcript tail — up to `max_lines` most recent screens, joined
    /// with screen separators. Empty when nothing was ever observed.
    #[must_use]
    pub fn transcript_tail(&self, max_lines: usize) -> String {
        if self.transcript.is_empty() {
            return String::new();
        }
        let skip = self.transcript.len().saturating_sub(max_lines);
        self.transcript
            .iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n── screen update ──\n")
    }
}

/// A styled snapshot of the visible screen's cells.
#[derive(Default, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenCells {
    /// Grid height in rows.
    pub rows: u16,
    /// Grid width in columns.
    pub cols: u16,
    /// Row-major cell grid, `rows * cols` entries.
    pub cells: Vec<TermCell>,
}

impl ScreenCells {
    /// The cell at `(row, col)`, or `None` when out of bounds.
    #[must_use]
    pub fn get(&self, row: u16, col: u16) -> Option<&TermCell> {
        let idx = usize::from(row) * usize::from(self.cols) + usize::from(col);
        self.cells.get(idx)
    }
}

/// One renderable cell of the terminal screen.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TermCell {
    /// The default cell (no contents, no styling).
    Blank,
    /// The right half of a double-width character.
    WideSpacer,
    /// A cell with content and optional styling.
    Styled {
        /// The character to draw.
        ch: char,
        /// Foreground/background/attributes.
        style: CellStyle,
    },
}

/// Foreground, background, and attribute styling of a cell.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct CellStyle {
    /// Foreground color.
    pub fg: TermColor,
    /// Background color.
    pub bg: TermColor,
    /// Bold attribute.
    pub bold: bool,
    /// Italic attribute.
    pub italic: bool,
    /// Underline attribute.
    pub underline: bool,
    /// Inverse attribute.
    pub inverse: bool,
}

/// Terminal color, normalized from vt100's palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TermColor {
    /// Terminal default foreground/background.
    #[default]
    Default,
    /// One of the 256-color palette entries.
    Idx(u8),
    /// A direct RGB color.
    Rgb(u8, u8, u8),
}

impl From<vt100::Color> for TermColor {
    fn from(color: vt100::Color) -> Self {
        match color {
            vt100::Color::Default => Self::Default,
            vt100::Color::Idx(idx) => Self::Idx(idx),
            vt100::Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    #[rstest::rstest]
    fn plain_text_renders_what_was_printed() {
        // Given a fresh emulator.
        let mut emu = Emulator::default();

        // When feeding plain printable output.
        emu.feed(b"hello world\r\nsecond line\r\n");

        // Then the plain text shows both lines without escape noise.
        let text = emu.plain_text();
        assert_eq!(text, "hello world\nsecond line");
    }

    #[rstest::rstest]
    fn ansi_styling_does_not_leak_into_plain_text() {
        // Given an emulator fed SGR-styled output.
        let mut emu = Emulator::default();

        // When feeding red bold text.
        emu.feed(b"\x1b[1;31mALERT\x1b[0m ok\r\n");

        // Then the plain text strips the escapes but keeps the words.
        assert_eq!(emu.plain_text(), "ALERT ok");
        // And the styled cell carries the red bold styling.
        let cells = emu.cells();
        let alert_cell = cells.get(0, 0).expect("cell (0,0)");
        let TermCell::Styled { ch: 'A', style } = alert_cell else {
            panic!("expected styled 'A', got {alert_cell:?}");
        };
        assert!(style.bold, "expected bold");
        assert_eq!(style.fg, TermColor::Idx(1), "expected palette red");
    }

    #[rstest::rstest]
    fn cursor_position_tracks_cursor_moves() {
        // Given an emulator fed a CUP (cursor position) sequence.
        let mut emu = Emulator::default();

        // When the program moves the cursor to row 3, col 5.
        emu.feed(b"\x1b[4;6H");

        // Then the cursor position is reported (0-indexed) as (3, 5).
        assert_eq!(emu.cursor_position(), (3, 5));
    }

    #[rstest::rstest]
    fn set_size_resizes_the_grid() {
        // Given a default-sized emulator.
        let mut emu = Emulator::default();

        // When resizing to 10x30.
        emu.set_size(10, 30);

        // Then the size reflects the change.
        assert_eq!(emu.size(), (10, 30));
        // And output longer than the new width wraps into the grid.
        emu.feed(b"012345678901234567890123456789X");
        let text = emu.plain_text();
        assert!(text.contains('X'), "wrapped content missing: {text:?}");
    }

    #[rstest::rstest]
    fn wide_character_occupies_two_cells() {
        // Given an emulator fed a double-width character (CJK).
        let mut emu = Emulator::default();

        // When printing '世' (U+4E16, double-width).
        emu.feed("世\n".as_bytes());

        // Then the leading cell holds the character.
        let cells = emu.cells();
        let lead = cells.get(0, 0).expect("cell (0,0)");
        let TermCell::Styled { ch: '世', .. } = lead else {
            panic!("expected wide lead cell, got {lead:?}");
        };
        // And the continuation cell is a wide spacer.
        assert_eq!(cells.get(0, 1), Some(&TermCell::WideSpacer));
    }

    #[rstest::rstest]
    fn zero_size_emulator_does_not_panic() {
        // Given an emulator constructed with a zeroed size (the pre-overlay
        // default that reached the pty before the (0,0) spawn fix).
        let mut emu = Emulator::new(0, 0, 4);

        // When it is fed bytes that fit the floor grid and resized to zero
        // again.
        emu.feed(b"hi\r\n");
        emu.set_size(0, 0);
        emu.sync_transcript();

        // Then the grid clamped to the 2x2 floor and content survives.
        assert_eq!(emu.size(), (2, 2));
        assert!(
            emu.plain_text().contains("hi"),
            "got: {:?}",
            emu.plain_text()
        );
    }

    #[rstest::rstest]
    fn wide_char_on_min_grid_does_not_panic() {
        // Given an emulator at the minimum 2x2 grid.
        let mut emu = Emulator::new(2, 2, 4);

        // When a double-width character is written (vt100 computes
        // `cols - width` with width = 2, panicking below the floor).
        emu.feed("\u{4e16}\u{4e16}\u{4e16}\r\n".as_bytes());

        // Then the emulator survived and holds the wide cell.
        let cells = emu.cells();
        assert_eq!(cells.rows, 2);
        assert!(matches!(
            cells.cells[0],
            TermCell::Styled { ch: '\u{4e16}', .. }
        ));
    }

    #[rstest::rstest]
    fn transcript_tail_keeps_most_recent_screens() {
        // Given an emulator whose transcript cap is two screens.
        let mut emu = Emulator::new(24, 80, 2);

        // When observing three successive screens with syncs between.
        emu.feed(b"screen one\r\n");
        emu.sync_transcript();
        emu.feed(b"\x1b[2J\x1b[Hscreen two\r\n");
        emu.sync_transcript();
        emu.feed(b"\x1b[2J\x1b[Hscreen three\r\n");
        emu.sync_transcript();

        // Then the tail of two keeps only the latest two screens.
        let tail = emu.transcript_tail(2);
        assert!(tail.contains("screen two"), "missing screen two: {tail}");
        assert!(
            tail.contains("screen three"),
            "missing screen three: {tail}"
        );
        assert!(
            !tail.contains("screen one"),
            "stale screen one kept: {tail}"
        );
    }

    #[rstest::rstest]
    fn transcript_sync_skips_repeated_identical_screens() {
        // Given an emulator that synced one screen.
        let mut emu = Emulator::default();
        emu.feed(b"stable screen\r\n");
        emu.sync_transcript();

        // When feeding nothing new and syncing again (a repaint with no
        // visible change).
        emu.sync_transcript();

        // Then the transcript holds exactly one entry.
        let tail = emu.transcript_tail(10);
        assert_eq!(tail.matches("stable screen").count(), 1);
    }

    #[rstest::rstest]
    fn transcript_tail_is_empty_before_any_sync() {
        // Given a fresh emulator.
        let emu = Emulator::default();

        // Then the transcript tail is empty.
        assert!(emu.transcript_tail(10).is_empty());
    }

    #[rstest::rstest]
    fn alternate_screen_contents_do_not_destroy_transcript() {
        // Given an emulator that observed a primary-screen line.
        let mut emu = Emulator::default();
        emu.feed(b"before tui\r\n");
        emu.sync_transcript();

        // When a fullscreen app takes the alternate screen, paints, and
        // leaves (as vim does: ?1049h … ?1049l).
        emu.feed(b"\x1b[?1049h\x1b[2J\x1b[Hvim screen\x1b[?1049l");

        // Then the transcript still contains the pre-TUI observation.
        let tail = emu.transcript_tail(10);
        assert!(
            tail.contains("before tui"),
            "lost pre-TUI transcript: {tail}"
        );
    }
}
