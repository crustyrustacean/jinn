//! Line-based selection highlight applied to the terminal buffer after rendering.

use ratatui::buffer::Buffer;

use crate::TuiApp;
use crate::selection::{SelectionState, find_last_nonws_in_row};

/// Applies line-based selection highlight to the buffer.
///
/// Row classification:
/// - **Single line** (anchor_y == focus_y): column-based highlight from min(ax,fx) to max(ax,fx).
/// - **First line** (anchor row): from anchor_x to last non-whitespace char.
/// - **Middle lines**: from bounds.x to last non-whitespace char.
/// - **Last line** (focus row): from bounds.x to focus_x.
///
/// For first and middle rows, the highlight extends from start_x to the last
/// non-whitespace character — internal spaces between words are included.
pub(super) fn apply_selection_highlight(app: &TuiApp, buf: &mut Buffer) {
    let (anchor, focus, bounds) = match app.selection {
        SelectionState::Dragging {
            anchor,
            focus,
            bounds,
        }
        | SelectionState::Active {
            anchor,
            focus,
            bounds,
        } => (anchor, focus, bounds),
        SelectionState::Idle => return,
    };

    let bounds_right = bounds.right().saturating_sub(1);
    let anchor_x = anchor.0.clamp(bounds.x, bounds_right);
    let focus_x = focus.0.clamp(bounds.x, bounds_right);
    let top_y = anchor.1.min(focus.1).max(bounds.y);
    let bot_y = anchor.1.max(focus.1).min(bounds.bottom().saturating_sub(1));

    // Extract theme colors once to avoid acquiring a read lock per cell.
    let (sel_fg, sel_bg) = {
        let state = app.core.state.read();
        (
            state.frontend.theme.selection_fg,
            state.frontend.theme.selection_bg,
        )
    };

    for y in top_y..=bot_y {
        let (start_x, end_x) = if top_y == bot_y {
            // Single line — column selection.
            (anchor_x.min(focus_x), anchor_x.max(focus_x))
        } else if y == anchor.1 {
            // First line — from anchor_x to last non-whitespace.
            let end = find_last_nonws_in_row(buf, y, anchor_x, bounds_right).unwrap_or(anchor_x);
            (anchor_x, end)
        } else if y == focus.1 {
            // Last line — from bounds.x to focus_x.
            (bounds.x, focus_x)
        } else {
            // Middle line — from bounds.x to last non-whitespace.
            let end = find_last_nonws_in_row(buf, y, bounds.x, bounds_right).unwrap_or(bounds.x);
            (bounds.x, end)
        };

        for x in start_x..=end_x {
            if let Some(cell) = buf.cell_mut((x, y)) {
                let fg = cell.fg;
                let bg = cell.bg;
                if fg == bg {
                    // Swapping identical colors is invisible
                    // (e.g. both Reset — the default for user messages
                    // and empty cells). Use explicit highlight colors.
                    cell.set_fg(sel_fg);
                    cell.set_bg(sel_bg);
                } else {
                    cell.set_fg(bg);
                    cell.set_bg(fg);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::SelectionState;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::style::Modifier;

    /// Creates a minimal `TuiApp` for render testing.
    fn render_test_app() -> crate::TuiApp {
        crate::TuiApp::test_builder().build()
    }

    #[rstest::rstest]
    fn cell_inside_selection_is_inverted() {
        // Given a buffer with distinctively colored cells and an active selection.
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // Paint a cell inside the selection with known colors and non-whitespace symbol.
        buf.cell_mut((3, 3)).unwrap().set_symbol("X");
        buf.cell_mut((3, 3)).unwrap().set_fg(Color::Yellow);
        buf.cell_mut((3, 3)).unwrap().set_bg(Color::Blue);
        // Paint a cell outside the selection with known colors.
        buf.cell_mut((15, 8)).unwrap().set_fg(Color::Red);
        buf.cell_mut((15, 8)).unwrap().set_bg(Color::Green);

        // And an app with an Active selection covering (2,2) to (5,4).
        let mut app = render_test_app();
        app.selection = SelectionState::Active {
            anchor: (2, 2),
            focus: (5, 4),
            bounds: area,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then cell (3, 3) inside the selection has swapped fg/bg.
        let inside = buf.cell((3, 3)).expect("cell inside selection");
        assert_eq!(inside.fg, Color::Blue); // was bg
        assert_eq!(inside.bg, Color::Yellow); // was fg
    }

    #[rstest::rstest]
    fn cell_outside_selection_is_unchanged() {
        // Given a buffer with distinctively colored cells and an active selection.
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // Paint a cell inside the selection with known colors.
        buf.cell_mut((3, 3)).unwrap().set_fg(Color::Yellow);
        buf.cell_mut((3, 3)).unwrap().set_bg(Color::Blue);
        // Paint a cell outside the selection with known colors.
        buf.cell_mut((15, 8)).unwrap().set_fg(Color::Red);
        buf.cell_mut((15, 8)).unwrap().set_bg(Color::Green);

        // And an app with an Active selection covering (2,2) to (5,4).
        let mut app = render_test_app();
        app.selection = SelectionState::Active {
            anchor: (2, 2),
            focus: (5, 4),
            bounds: area,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then cell (15, 8) outside the selection is unchanged.
        let outside = buf.cell((15, 8)).expect("cell outside selection");
        assert_eq!(outside.fg, Color::Red);
        assert_eq!(outside.bg, Color::Green);
    }

    #[rstest::rstest]
    fn cell_inside_clamped_selection_is_inverted() {
        // Given a buffer covering a large area and a selection where the raw anchor
        // extends beyond the selection's constraining bounds.
        let full_area = Rect::new(0, 0, 30, 30);
        let mut buf = ratatui::buffer::Buffer::empty(full_area);
        // Paint cell inside bounds (will be in clamped selection) with non-whitespace symbol.
        buf.cell_mut((7, 7)).unwrap().set_symbol("X");
        buf.cell_mut((7, 7)).unwrap().set_fg(Color::Cyan);
        buf.cell_mut((7, 7)).unwrap().set_bg(Color::Magenta);
        // Paint cell at raw anchor position (0, 0) — outside bounds.
        buf.cell_mut((0, 0)).unwrap().set_fg(Color::White);
        buf.cell_mut((0, 0)).unwrap().set_bg(Color::Black);

        // And an Active selection with anchor outside bounds.
        // bounds=(5,5,10,10) means valid range is (5,5)-(14,14).
        // anchor=(0,0) is outside bounds, focus=(8,8) is inside.
        // selection_rect() should clamp to (5,5)-(8,8).
        let bounds = Rect::new(5, 5, 10, 10);
        let mut app = render_test_app();
        app.selection = SelectionState::Active {
            anchor: (0, 0),
            focus: (8, 8),
            bounds,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then cell (7, 7) inside the clamped selection is inverted.
        let inside = buf.cell((7, 7)).expect("cell inside clamped selection");
        assert_eq!(inside.fg, Color::Magenta); // was bg
        assert_eq!(inside.bg, Color::Cyan); // was fg
    }

    #[rstest::rstest]
    fn cell_at_raw_anchor_not_inverted() {
        // Given a buffer covering a large area and a selection where the raw anchor
        // extends beyond the selection's constraining bounds.
        let full_area = Rect::new(0, 0, 30, 30);
        let mut buf = ratatui::buffer::Buffer::empty(full_area);
        // Paint cell inside bounds (will be in clamped selection).
        buf.cell_mut((7, 7)).unwrap().set_fg(Color::Cyan);
        buf.cell_mut((7, 7)).unwrap().set_bg(Color::Magenta);
        // Paint cell at raw anchor position (0, 0) — outside bounds.
        buf.cell_mut((0, 0)).unwrap().set_fg(Color::White);
        buf.cell_mut((0, 0)).unwrap().set_bg(Color::Black);

        // And an Active selection with anchor outside bounds.
        // bounds=(5,5,10,10) means valid range is (5,5)-(14,14).
        // anchor=(0,0) is outside bounds, focus=(8,8) is inside.
        // selection_rect() should clamp to (5,5)-(8,8).
        let bounds = Rect::new(5, 5, 10, 10);
        let mut app = render_test_app();
        app.selection = SelectionState::Active {
            anchor: (0, 0),
            focus: (8, 8),
            bounds,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then cell (0, 0) at the raw anchor position is NOT inverted.
        let outside = buf.cell((0, 0)).expect("cell at raw anchor");
        assert_eq!(outside.fg, Color::White); // unchanged
        assert_eq!(outside.bg, Color::Black); // unchanged
    }

    #[rstest::rstest]
    fn highlight_does_nothing_when_idle() {
        // Given a buffer with distinctly colored cells and an Idle selection.
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        buf.cell_mut((5, 5)).unwrap().set_fg(Color::Yellow);
        buf.cell_mut((5, 5)).unwrap().set_bg(Color::Blue);

        // And an app with an Idle selection.
        let mut app = render_test_app();
        app.selection = SelectionState::Idle;

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then no cells are inverted — colors remain unchanged.
        let cell = buf.cell((5, 5)).expect("colored cell");
        assert_eq!(cell.fg, Color::Yellow); // unchanged
        assert_eq!(cell.bg, Color::Blue); // unchanged
    }

    #[rstest::rstest]
    fn reset_bg_cell_gets_explicit_colors() {
        // Given a buffer where cells have matching fg and bg (e.g. both Reset,
        // as with user messages rendered with Style::default().bold()).
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // User-message-style cell: fg = Reset, bg = Reset (bold modifier).
        buf.cell_mut((3, 3))
            .unwrap()
            .set_style(ratatui::style::Style::default().add_modifier(Modifier::BOLD));
        buf.cell_mut((3, 3)).unwrap().set_symbol("X");
        // Adjacent cell with distinct colors (assistant-style).
        buf.cell_mut((4, 3)).unwrap().set_symbol("Y");
        buf.cell_mut((4, 3)).unwrap().set_fg(Color::Cyan);

        // And an Active selection covering both cells.
        let mut app = render_test_app();
        app.selection = SelectionState::Active {
            anchor: (2, 2),
            focus: (5, 4),
            bounds: area,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then the Reset/Reset cell gets explicit highlight colors.
        let reset_cell = buf.cell((3, 3)).expect("reset cell");
        assert_eq!(reset_cell.fg, Color::Black);
        assert_eq!(reset_cell.bg, Color::White);
    }

    #[rstest::rstest]
    fn distinct_color_cell_gets_swapped() {
        // Given a buffer where cells have matching fg and bg (e.g. both Reset,
        // as with user messages rendered with Style::default().bold()).
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // User-message-style cell: fg = Reset, bg = Reset (bold modifier).
        buf.cell_mut((3, 3))
            .unwrap()
            .set_style(ratatui::style::Style::default().add_modifier(Modifier::BOLD));
        buf.cell_mut((3, 3)).unwrap().set_symbol("X");
        // Adjacent cell with distinct colors (assistant-style).
        buf.cell_mut((4, 3)).unwrap().set_symbol("Y");
        buf.cell_mut((4, 3)).unwrap().set_fg(Color::Cyan);

        // And an Active selection covering both cells.
        let mut app = render_test_app();
        app.selection = SelectionState::Active {
            anchor: (2, 2),
            focus: (5, 4),
            bounds: area,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then the distinct-colors cell gets swapped fg/bg.
        let cyan_cell = buf.cell((4, 3)).expect("cyan cell");
        assert_eq!(cyan_cell.fg, Color::Reset); // was bg
        assert_eq!(cyan_cell.bg, Color::Cyan); // was fg
    }
}
