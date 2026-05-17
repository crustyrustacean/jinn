//! Tests for [`SelectionWidget`] and [`compute_popup_rect`].

use nullslop_testutil::setup_term;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};

use crate::{PickerItem, SelectionState, SelectionWidget, compute_popup_rect, widget::PROMPT};

/// A minimal item type for testing.
#[derive(Debug)]
struct TestItem {
    label: String,
}

impl TestItem {
    fn new(label: &str) -> Self {
        Self {
            label: label.to_owned(),
        }
    }
}

impl PickerItem for TestItem {
    fn display_label(&self) -> &str {
        &self.label
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        if is_selected {
            Line::from(format!("> {}", self.label))
        } else {
            Line::from(self.label.clone())
        }
    }
}

/// Creates a list of test items from the given labels.
fn make_items(labels: &[&str]) -> Vec<TestItem> {
    labels.iter().map(|&l| TestItem::new(l)).collect()
}

// =========================================================================
// Ported from render.rs provider picker tests
// =========================================================================

#[rstest::rstest]
fn render_shows_telescope_layout() {
    // Given a selection state with filter text.
    let mut state = SelectionState::with_items(make_items(&["ollama"]));
    state.insert_char('o');
    state.insert_char('l');

    let (mut terminal, _) = setup_term(80, 24);

    // When rendering the widget.
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state);
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the popup contains the filter prompt "> " and separator "─".
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    // Filter is on the first inner row: popup.y + 1
    let filter_y = popup.y + 1;
    let prompt_cell = buffer.cell((popup.x + 1, filter_y)).expect("prompt cell");
    assert_eq!(prompt_cell.symbol(), ">");

    // Separator is on the second inner row.
    let sep_y = popup.y + 2;
    let sep_cell = buffer.cell((popup.x + 1, sep_y)).expect("sep cell");
    assert_eq!(sep_cell.symbol(), "\u{2500}");
}

#[rstest::rstest]
fn larger_terminal_taller_popup() {
    // Given two terminal sizes.
    let small_area = Rect::new(0, 0, 80, 24);
    let large_area = Rect::new(0, 0, 80, 42);

    // When computing popup rects.
    let small_popup = compute_popup_rect(small_area);
    let large_popup = compute_popup_rect(large_area);

    // Then the larger terminal gets a taller popup.
    assert!(large_popup.height > small_popup.height);
}

#[rstest::rstest]
fn small_terminal_uses_75_percent() {
    // Given a small terminal.
    let small_area = Rect::new(0, 0, 80, 24);

    // When computing popup rect.
    let small_popup = compute_popup_rect(small_area);

    // Then the popup uses 75% of height + 4 rows of chrome.
    // floor(24 * 0.75) = 18, min(18 + 4, 24) = 22.
    assert_eq!(small_popup.height, 22);
}

#[rstest::rstest]
fn render_uses_dark_gray_border() {
    // Given a widget with default state.
    let state = SelectionState::with_items(make_items(&["test"]));

    let (mut terminal, _) = setup_term(80, 24);

    // When rendering the widget.
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state);
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the border color is DarkGray.
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let border_cell = buffer.cell((popup.x, popup.y)).expect("border cell");
    assert_eq!(border_cell.fg, Color::DarkGray);
}

#[rstest::rstest]
fn render_calls_render_row_for_selected_item() {
    // Given a selection state with items where the first is selected.
    let state = SelectionState::with_items(make_items(&["alpha", "bravo"]));

    let (mut terminal, _) = setup_term(80, 24);

    // When rendering the widget.
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state);
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the first result row starts with "> " (render_row with is_selected=true).
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    // Results start at popup.y + 3 (border + input + separator)
    let result_y = popup.y + 3;
    let marker_cell = buffer.cell((popup.x + 1, result_y)).expect("marker cell");
    assert_eq!(marker_cell.symbol(), ">");
}

// =========================================================================
// New widget-specific tests
// =========================================================================

#[rstest::rstest]
fn render_shows_title() {
    // Given a widget with a custom title.
    let state = SelectionState::with_items(make_items(&["test"]));
    let (mut terminal, _) = setup_term(80, 24);

    // When rendering with title " Model ".
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state).title(Line::from(" Model "));
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the title appears in the border area.
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    // Title is rendered on the top border row. ratatui renders the title
    // starting at a position along the top border line.
    let title_y = popup.y;
    // Find the 'M' from " Model " on the top border row.
    let mut found_title = false;
    for col in popup.x..popup.x + popup.width {
        if let Some(cell) = buffer.cell((col, title_y))
            && cell.symbol() == "M"
        {
            found_title = true;
            break;
        }
    }
    assert!(
        found_title,
        "expected to find 'M' from title ' Model ' on the top border row"
    );
}

#[rstest::rstest]
fn render_shows_footer_when_provided() {
    // Given a widget with a footer.
    let state = SelectionState::with_items(make_items(&["test"]));
    let (mut terminal, _) = setup_term(80, 24);

    // When rendering with footer text.
    let footer_text = "CTRL+R to refresh";
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state).footer(Line::from(footer_text));
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the footer content appears in the footer area.
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let inner = {
        let b = Block::default().borders(Borders::ALL);
        b.inner(popup)
    };
    // Footer is the last row of the inner area.
    let footer_y = inner.y + inner.height - 1;

    // Footer is right-aligned, so search for "C" from "CTRL" somewhere on footer_y.
    let mut found_footer = false;
    for col in inner.x..inner.x + inner.width {
        if let Some(cell) = buffer.cell((col, footer_y))
            && cell.symbol() == "C"
        {
            found_footer = true;
            break;
        }
    }
    assert!(
        found_footer,
        "expected to find footer text on the footer row"
    );
}

#[rstest::rstest]
fn render_no_footer_shows_empty_row() {
    // Given a widget without a footer.
    let state = SelectionState::with_items(make_items(&["test"]));
    let (mut terminal, _) = setup_term(80, 24);

    // When rendering without footer.
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state);
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the footer area contains empty/spaces (no visible text).
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let inner = {
        let b = Block::default().borders(Borders::ALL);
        b.inner(popup)
    };
    let footer_y = inner.y + inner.height - 1;

    // All cells in the footer row should be spaces (empty).
    for col in inner.x..inner.x + inner.width {
        if let Some(cell) = buffer.cell((col, footer_y)) {
            assert_eq!(
                cell.symbol(),
                " ",
                "expected empty cell at ({}, {}), got '{}'",
                col,
                footer_y,
                cell.symbol()
            );
        }
    }
}

#[rstest::rstest]
fn render_pads_empty_result_rows() {
    // Given a selection state with only 1 item but many visible rows.
    let state = SelectionState::with_items(make_items(&["solo"]));
    let (mut terminal, _) = setup_term(80, 24);

    // When rendering the widget.
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state);
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the results area is mostly empty rows (only the first result has content).
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let inner = {
        let b = Block::default().borders(Borders::ALL);
        b.inner(popup)
    };
    // Results area starts at inner.y + 2 (after input and separator).
    let results_start_y = inner.y + 2;
    // Results area height = inner.height - 3 (input + separator + footer).
    let results_height = inner.height - 3;

    // First result row should have content.
    let first_cell = buffer
        .cell((inner.x, results_start_y))
        .expect("first result cell");
    // The item is "solo" and is selected, so render_row returns "> solo".
    assert_ne!(
        first_cell.symbol(),
        " ",
        "first result row should have content"
    );

    // Second result row should be empty (padded).
    if results_height > 1 {
        let second_cell = buffer
            .cell((inner.x, results_start_y + 1))
            .expect("second result cell");
        assert_eq!(
            second_cell.symbol(),
            " ",
            "second result row should be empty/padded"
        );
    }
}

#[rstest::rstest]
fn render_clears_popup_background() {
    // Given a selection state with items and a pre-filled buffer.
    let state = SelectionState::with_items(make_items(&["test"]));
    let (mut terminal, _) = setup_term(80, 24);

    // Pre-fill the buffer with 'X' characters so we can verify clearing.
    terminal
        .draw(|frame| {
            let area = frame.area();
            let x_text = "X".repeat(area.width as usize);
            for row in 0..area.height {
                frame.render_widget(
                    ratatui::widgets::Paragraph::new(x_text.clone()),
                    ratatui::layout::Rect::new(0, row, area.width, 1),
                );
            }
        })
        .unwrap();

    // When rendering the widget.
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state);
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the popup interior cells are cleared (not 'X').
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let inner = {
        let b = Block::default().borders(Borders::ALL);
        b.inner(popup)
    };
    // Check a cell in the middle of the inner area — should be space, not 'X'.
    let mid_x = inner.x + inner.width / 2;
    let mid_y = inner.y + inner.height / 2;
    let cell = buffer.cell((mid_x, mid_y)).expect("mid cell");
    assert_ne!(
        cell.symbol(),
        "X",
        "popup interior should be cleared, not showing previous content"
    );
}

#[rstest::rstest]
fn render_positions_cursor_correctly() {
    // Given a selection state with filter text and cursor in the middle.
    let mut state = SelectionState::with_items(make_items(&["test"]));
    state.insert_char('a');
    state.insert_char('b');
    state.insert_char('c');
    // Filter is "abc", cursor at position 3 (end).
    // Move cursor back to position 1 (between 'a' and 'b').
    state.move_cursor_left();
    state.move_cursor_left();
    assert_eq!(state.cursor_pos(), 1);

    let (mut terminal, _) = setup_term(80, 24);

    // When rendering the widget.
    terminal
        .draw(|frame| {
            let widget = SelectionWidget::new(&state);
            widget.render(frame, frame.area());
        })
        .unwrap();

    // Then the cursor is at the expected position within the input row.
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let inner = {
        let b = Block::default().borders(Borders::ALL);
        b.inner(popup)
    };
    // Cursor should be at: input_area.x + PROMPT.len() + cursor_pos = inner.x + 2 + 1 = inner.x + 3
    let expected_cursor_x = inner.x + (PROMPT.len() + 1) as u16;
    let expected_cursor_y = inner.y;

    // Verify the buffer cell at the cursor position shows 'b' (cursor is before 'b').
    let buffer = terminal.backend().buffer().clone();
    let cursor_cell = buffer
        .cell((expected_cursor_x, expected_cursor_y))
        .expect("cursor cell");
    assert_eq!(cursor_cell.symbol(), "b");
}

#[rstest::rstest]
fn compute_popup_rect_width_clamps_to_min() {
    // Given a very narrow terminal (width 20, less than PICKER_MIN_WIDTH).
    let area = Rect::new(0, 0, 20, 24);

    // When computing the popup rect.
    let popup = compute_popup_rect(area);

    // Then popup width is PICKER_MIN_WIDTH, but cannot exceed area width.
    assert_eq!(
        popup.width,
        crate::PICKER_MIN_WIDTH.min(area.width),
        "popup width should be clamped to min or terminal width"
    );
}

#[rstest::rstest]
fn popup_width_does_not_exceed_terminal() {
    // Given a terminal area.
    let area = Rect::new(0, 0, 80, 24);

    // When computing the popup rect.
    let popup = compute_popup_rect(area);

    // Then popup width never exceeds terminal width.
    assert!(popup.width <= area.width, "popup width exceeds terminal");
}

#[rstest::rstest]
fn popup_height_does_not_exceed_terminal() {
    // Given a terminal area.
    let area = Rect::new(0, 0, 80, 24);

    // When computing the popup rect.
    let popup = compute_popup_rect(area);

    // Then popup height never exceeds terminal height.
    assert!(popup.height <= area.height, "popup height exceeds terminal");
}

#[rstest::rstest]
fn compute_popup_rect_centers_horizontally() {
    // Given an 80-wide terminal.
    let area = Rect::new(0, 0, 80, 24);

    // When computing the popup rect.
    let popup = compute_popup_rect(area);

    // Then popup is horizontally centered (equal padding on both sides).
    let left_pad = popup.x;
    let right_pad = area.width - (popup.x + popup.width);
    // Allow off-by-one due to integer division.
    assert!(
        (i32::from(left_pad) - i32::from(right_pad)).unsigned_abs() <= 1,
        "popup should be roughly centered: left_pad={left_pad}, right_pad={right_pad}"
    );
}

#[rstest::rstest]
fn compute_popup_rect_biased_to_top_third() {
    // Given a tall terminal.
    let area = Rect::new(0, 0, 80, 60);

    // When computing the popup rect.
    let popup = compute_popup_rect(area);

    // Then popup is positioned in the top third (y < height / 3).
    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let area_third = area.height / 3;
    assert!(
        popup.y < area_third,
        "popup y ({}) should be in the top third (below {})",
        popup.y,
        area_third
    );
}
