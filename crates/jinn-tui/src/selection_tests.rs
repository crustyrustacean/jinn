#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test file, panics are acceptable"
)]

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::selection::{SelectableRects, SelectionState};

fn bounds() -> Rect {
    Rect::new(0, 0, 20, 20)
}

#[rstest::rstest]
fn start_drag_creates_dragging_state() {
    // Given no prior state.
    // When starting a drag at (5, 7) within bounds.
    let state = SelectionState::start_drag(5, 7, bounds());

    // Then the state is Dragging with anchor and focus at (5, 7).
    assert_eq!(
        state,
        SelectionState::Dragging {
            anchor: (5, 7),
            focus: (5, 7),
            bounds: bounds(),
        }
    );
}

#[rstest::rstest]
fn update_focus_clamps_to_bounds() {
    // Given a Dragging state at (5, 5) with bounds (0, 0, 10, 10).
    let state = SelectionState::start_drag(5, 5, Rect::new(0, 0, 10, 10));

    // When updating focus to (15, 15) which exceeds bounds.
    let state = state.update_focus(15, 15);

    // Then the focus is clamped to (9, 9).
    assert_eq!(state.focus(), Some((9, 9)));
}

#[rstest::rstest]
fn finalize_transitions_dragging_to_active() {
    // Given a Dragging state.
    let state = SelectionState::start_drag(1, 2, bounds()).update_focus(5, 6);

    // When finalizing.
    let state = state.finalize();

    // Then the state is Active with the same anchor, focus, and bounds.
    assert_eq!(
        state,
        SelectionState::Active {
            anchor: (1, 2),
            focus: (5, 6),
            bounds: bounds(),
        }
    );
}

#[rstest::rstest]
fn cancel_returns_to_idle() {
    // Given a Dragging state.
    let state = SelectionState::start_drag(3, 4, bounds());

    // When cancelling.
    let state = state.cancel();

    // Then the state is Idle.
    assert_eq!(state, SelectionState::Idle);
}

#[rstest::rstest]
fn idle_returns_none_for_accessors() {
    // Given an Idle state.
    let state = SelectionState::Idle;

    // Then all accessors return None.
    assert!(state.anchor().is_none());
    assert!(state.focus().is_none());
    assert!(state.bounds().is_none());
}

#[rstest::rstest]
fn idle_returns_none_for_extract_text() {
    // Given an Idle state and an empty buffer.
    let state = SelectionState::Idle;
    let buffer = Buffer::empty(Rect::new(0, 0, 10, 5));

    // When extracting text.
    // Then it returns None.
    assert!(state.extract_text(&buffer).is_none());
}

#[rstest::rstest]
fn single_row_selection_returns_text() {
    // Given a buffer with known text on row 2.
    let area = Rect::new(0, 0, 10, 5);
    let mut buffer = Buffer::empty(area);
    // Write "Hello" starting at (2, 2).
    for (i, ch) in "Hello".chars().enumerate() {
        let cell = buffer.cell_mut((2 + i as u16, 2)).unwrap();
        cell.set_symbol(&ch.to_string());
    }

    // And an Active selection covering cells (2,2) to (6,2).
    let state = SelectionState::Active {
        anchor: (2, 2),
        focus: (6, 2),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer);

    // Then text is returned (not None).
    assert!(text.is_some());
}

#[rstest::rstest]
fn single_row_selection_returns_hello() {
    // Given a buffer with known text on row 2.
    let area = Rect::new(0, 0, 10, 5);
    let mut buffer = Buffer::empty(area);
    // Write "Hello" starting at (2, 2).
    for (i, ch) in "Hello".chars().enumerate() {
        let cell = buffer.cell_mut((2 + i as u16, 2)).unwrap();
        cell.set_symbol(&ch.to_string());
    }

    // And an Active selection covering cells (2,2) to (6,2).
    let state = SelectionState::Active {
        anchor: (2, 2),
        focus: (6, 2),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer).expect("should return text");

    // Then the text matches the content of those cells.
    assert_eq!(text, "Hello");
}

#[rstest::rstest]
fn multi_row_selection_spans_rows() {
    // Given a buffer with text on two rows.
    let area = Rect::new(0, 0, 10, 5);
    let mut buffer = Buffer::empty(area);
    // Row 1: "AB" at (0, 1) and (1, 1).
    buffer.cell_mut((0, 1)).unwrap().set_symbol("A");
    buffer.cell_mut((1, 1)).unwrap().set_symbol("B");
    // Row 2: "CD" at (0, 2) and (1, 2).
    buffer.cell_mut((0, 2)).unwrap().set_symbol("C");
    buffer.cell_mut((1, 2)).unwrap().set_symbol("D");

    // And an Active selection spanning rows 1 and 2.
    let state = SelectionState::Active {
        anchor: (0, 1),
        focus: (1, 2),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer);

    // Then text is returned (not None).
    assert!(text.is_some());
}

#[rstest::rstest]
fn rows_joined_with_newline() {
    // Given a buffer with text on two rows.
    let area = Rect::new(0, 0, 10, 5);
    let mut buffer = Buffer::empty(area);
    // Row 1: "AB" at (0, 1) and (1, 1).
    buffer.cell_mut((0, 1)).unwrap().set_symbol("A");
    buffer.cell_mut((1, 1)).unwrap().set_symbol("B");
    // Row 2: "CD" at (0, 2) and (1, 2).
    buffer.cell_mut((0, 2)).unwrap().set_symbol("C");
    buffer.cell_mut((1, 2)).unwrap().set_symbol("D");

    // And an Active selection spanning rows 1 and 2.
    let state = SelectionState::Active {
        anchor: (0, 1),
        focus: (1, 2),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer).expect("should return text");

    // Then the rows are joined with newline.
    assert_eq!(text, "AB\nCD");
}

#[rstest::rstest]
fn accessors_return_positions_for_active_state() {
    // Given an Active state where anchor (5, 5) is after focus (2, 2).
    let state = SelectionState::Active {
        anchor: (5, 5),
        focus: (2, 2),
        bounds: bounds(),
    };

    // Then the accessors return the raw positions.
    assert_eq!(state.anchor(), Some((5, 5)));
    assert_eq!(state.focus(), Some((2, 2)));
    assert_eq!(state.bounds(), Some(bounds()));
}

// --- Line selection tests ---

#[rstest::rstest]
fn first_line_starts_at_anchor_x() {
    // Given a buffer with "ABCDE" on row 1 starting at col 0.
    let area = Rect::new(0, 0, 10, 5);
    let mut buffer = Buffer::empty(area);
    for (i, ch) in "ABCDE".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 1))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    // And a 3-row selection starting at anchor (3, 1) going to focus (4, 3).
    let state = SelectionState::Active {
        anchor: (3, 1),
        focus: (4, 3),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer).expect("should return text");

    // Then the first line starts at anchor_x=3, so "DE".
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[0], "DE");
}

#[rstest::rstest]
fn middle_line_selects_to_last_nonws() {
    // Given a buffer with "hello   world!" on row 2 (with spaces between).
    let area = Rect::new(0, 0, 15, 5);
    let mut buffer = Buffer::empty(area);
    for (i, ch) in "hello   world!".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 2))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    // And a 3-row selection (rows 1-3), row 2 is a middle row.
    let state = SelectionState::Active {
        anchor: (0, 1),
        focus: (5, 3),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer).expect("should return text");

    // Then the middle row includes spaces between words.
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[1], "hello   world!");
}

#[rstest::rstest]
fn last_line_stops_at_focus_x() {
    // Given a buffer with "ABCDEFGHIJ" on row 3.
    let area = Rect::new(0, 0, 15, 5);
    let mut buffer = Buffer::empty(area);
    for (i, ch) in "ABCDEFGHIJ".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 3))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    // And a 3-row selection (rows 1-3), focus_x = 5.
    let state = SelectionState::Active {
        anchor: (0, 1),
        focus: (5, 3),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer).expect("should return text");

    // Then the last line stops at focus_x=5 (inclusive), so "ABCDEF".
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[2], "ABCDEF");
}

#[rstest::rstest]
fn backward_selection_matches_forward_behavior() {
    // Given a buffer with text on rows 1-3.
    let area = Rect::new(0, 0, 10, 5);
    let mut buffer = Buffer::empty(area);
    for (i, ch) in "ABCDE".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 1))
            .unwrap()
            .set_symbol(&ch.to_string());
    }
    for (i, ch) in "FGHIJ".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 2))
            .unwrap()
            .set_symbol(&ch.to_string());
    }
    for (i, ch) in "KLMNO".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 3))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    // And a backward selection (anchor at row 3, focus at row 1).
    let state = SelectionState::Active {
        anchor: (3, 3),
        focus: (1, 1),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer).expect("should return text");

    // Then the result matches the equivalent forward selection.
    // Top row (y=1) has "ABCDE": from top_x=1 to last nonws → "BCDE"
    // Middle row (y=2) has "FGHIJ": from bounds.x=0 to last nonws → "FGHIJ"
    // Bottom row (y=3) has "KLMNO": from bounds.x=0 to bot_x=3 → "KLMN"
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[0], "BCDE");
    assert_eq!(lines[1], "FGHIJ");
    assert_eq!(lines[2], "KLMN");
}

#[rstest::rstest]
fn single_line_selection_is_column_based() {
    // Given a buffer with "ABCDE" on row 2.
    let area = Rect::new(0, 0, 10, 5);
    let mut buffer = Buffer::empty(area);
    for (i, ch) in "ABCDE".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 2))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    // And a single-row selection from col 1 to col 3.
    let state = SelectionState::Active {
        anchor: (1, 2),
        focus: (3, 2),
        bounds: area,
    };

    // When extracting text.
    let text = state.extract_text(&buffer).expect("should return text");

    // Then only the columns between anchor_x and focus_x are selected.
    assert_eq!(text, "BCD");
}

#[rstest::rstest]
fn backward_selection_extracts_same_text_as_forward() {
    // Given a buffer with text on rows 1-3.
    let area = Rect::new(0, 0, 10, 5);
    let mut buffer = Buffer::empty(area);
    for (i, ch) in "ABCDE".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 1))
            .unwrap()
            .set_symbol(&ch.to_string());
    }
    for (i, ch) in "FGHIJ".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 2))
            .unwrap()
            .set_symbol(&ch.to_string());
    }
    for (i, ch) in "KLMNO".chars().enumerate() {
        buffer
            .cell_mut((i as u16, 3))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    // And two selections covering the same rows but in opposite directions.
    let forward = SelectionState::Active {
        anchor: (1, 1),
        focus: (3, 3),
        bounds: area,
    };
    let backward = SelectionState::Active {
        anchor: (3, 3),
        focus: (1, 1),
        bounds: area,
    };

    // When extracting text from both.
    let forward_text = forward
        .extract_text(&buffer)
        .expect("forward should return text");
    let backward_text = backward
        .extract_text(&buffer)
        .expect("backward should return text");

    // Then the extracted text is identical regardless of drag direction.
    assert_eq!(forward_text, backward_text);
}

// --- SelectableRects tests ---

#[rstest::rstest]
fn selectable_rects_find_returns_smallest_matching() {
    // Given overlapping rects - a large screen and a smaller pane.
    let screen = Rect::new(0, 0, 80, 24);
    let pane = Rect::new(10, 5, 20, 10);
    let mut rects = SelectableRects::new();
    rects.rebuild(vec![screen, pane]);

    // When querying a position inside the pane.
    let found = rects.find_for_position(15, 8);

    // Then the smaller pane rect is returned.
    assert_eq!(found, Some(pane));
}

#[rstest::rstest]
fn selectable_rects_find_returns_none_for_position_outside_all() {
    // Given a single rect.
    let mut rects = SelectableRects::new();
    rects.rebuild(vec![Rect::new(0, 0, 10, 10)]);

    // When querying a position outside the rect.
    let found = rects.find_for_position(20, 20);

    // Then None is returned.
    assert_eq!(found, None);
}

#[rstest::rstest]
fn selectable_rects_find_returns_none_when_empty() {
    // Given no rects registered.
    let rects = SelectableRects::new();

    // When querying any position.
    let found = rects.find_for_position(5, 5);

    // Then None is returned.
    assert_eq!(found, None);
}

#[rstest::rstest]
fn selectable_rects_rebuild_replaces_previous_rects() {
    // Given rects with an initial rect.
    let mut rects = SelectableRects::new();
    rects.rebuild(vec![Rect::new(0, 0, 10, 10)]);

    // When rebuilding with different rects.
    rects.rebuild(vec![Rect::new(20, 20, 5, 5)]);

    // Then the old rect is gone and only the new one matches.
    assert_eq!(rects.find_for_position(5, 5), None);
    assert_eq!(
        rects.find_for_position(22, 22),
        Some(Rect::new(20, 20, 5, 5))
    );
}

// --- Mutant-killing tests for selection.rs ---

// Kills: find_for_position < -> <= (both occurrences, line 44)
#[rstest::rstest]
fn find_for_position_excludes_right_and_bottom_edges() {
    // Rect at (0,0) with width=10, height=5.
    // right()=10, bottom()=5.
    // Point (10, 2) is on the right edge - must NOT match (x < right is false when x==right).
    // With <= it would incorrectly match.
    let mut rects = SelectableRects::new();
    let rect = Rect::new(0, 0, 10, 5);
    rects.rebuild(vec![rect]);

    // Right edge exclusion.
    assert_eq!(
        rects.find_for_position(10, 2),
        None,
        "x=right() must not match with <"
    );
    // Bottom edge exclusion.
    assert_eq!(
        rects.find_for_position(5, 5),
        None,
        "y=bottom() must not match with <"
    );
    // Interior point must match.
    assert_eq!(rects.find_for_position(5, 2), Some(rect));
}

// Kills: find_for_position * -> +, * -> /
#[rstest::rstest]
fn find_for_position_uses_multiplication_for_area() {
    // Two rects: one 2x3=6 area, one 3x2=6 area (tie).
    // And one 1x1=1 area inside both. The smallest must win.
    let large1 = Rect::new(0, 0, 3, 3); // area = 3*3 = 9
    let small = Rect::new(0, 0, 1, 1); // area = 1*1 = 1
    let mut rects = SelectableRects::new();
    rects.rebuild(vec![large1, small]);

    let found = rects.find_for_position(0, 0);
    assert_eq!(
        found,
        Some(small),
        "smallest area rect must win (area=1, not 9)"
    );
}

// Kills: find_last_nonws_in_row delete !
#[rstest::rstest]
fn find_last_nonws_returns_last_not_first_nonws() {
    use crate::selection::find_last_nonws_in_row;
    let area = Rect::new(0, 0, 10, 1);
    let mut buffer = Buffer::empty(area);
    // Cells: "A  B"
    buffer.cell_mut((0, 0)).unwrap().set_symbol("A");
    // (1,0) and (2,0) are whitespace
    buffer.cell_mut((3, 0)).unwrap().set_symbol("B");
    // (4,0) through (9,0) are whitespace

    let result = find_last_nonws_in_row(&buffer, 0, 0, 9);
    // Without the !, it would return the first non-ws (x=0).
    // With !, it returns the last non-ws (x=3).
    assert_eq!(
        result,
        Some(3),
        "must return last non-ws position (x=3), not first (x=0)"
    );
}

// Kills: SelectionState::cancel -> Default::default()
#[rstest::rstest]
fn cancel_dragging_returns_idle_variant() {
    let state = SelectionState::Dragging {
        anchor: (1, 2),
        focus: (3, 4),
        bounds: bounds(),
    };
    let cancelled = state.cancel();
    // Explicitly check it's Idle, not just that it's the default value.
    assert_eq!(cancelled, SelectionState::Idle);
    assert!(matches!(cancelled, SelectionState::Idle));
}

// Kills: SelectionState::is_active -> true
#[rstest::rstest]
fn idle_is_not_active() {
    assert!(
        !SelectionState::Idle.is_active(),
        "Idle must return false for is_active"
    );
}

#[rstest::rstest]
fn dragging_is_active() {
    let state = SelectionState::Dragging {
        anchor: (1, 2),
        focus: (3, 4),
        bounds: bounds(),
    };
    assert!(state.is_active(), "Dragging must return true for is_active");
}

#[rstest::rstest]
fn active_selection_is_active() {
    let state = SelectionState::Active {
        anchor: (1, 2),
        focus: (3, 4),
        bounds: bounds(),
    };
    assert!(state.is_active(), "Active must return true for is_active");
}
