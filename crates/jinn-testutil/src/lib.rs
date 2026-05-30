//! Shared test utilities for jinn TUI rendering tests.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

/// Creates a test terminal and full-area rect for the given dimensions.
///
/// # Panics
///
/// Panics if `Terminal::new` fails, which should only happen with zero-sized dimensions.
pub fn setup_term(width: u16, height: u16) -> (Terminal<TestBackend>, Rect) {
    let backend = TestBackend::new(width, height);
    let terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, width, height);
    (terminal, area)
}

/// Extracts a single row from a ratatui buffer as a `String`.
pub fn buffer_row(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    (0..width)
        .filter_map(|x| buffer.cell((x, y)).map(ratatui::buffer::Cell::symbol))
        .collect()
}

/// Extracts all rows from a ratatui buffer as `String`s.
pub fn buffer_rows(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) -> Vec<String> {
    (0..height).map(|y| buffer_row(buffer, y, width)).collect()
}
