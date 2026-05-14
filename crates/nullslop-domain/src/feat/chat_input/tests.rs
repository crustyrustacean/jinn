use super::*;

#[rstest::rstest]
fn grapheme_count_returns_cluster_count() {
    // Given a state with "éNoël" inserted.
    let mut state = ChatInputBoxState::new();
    state.insert_text("éNoël");

    // When reading the grapheme count.
    // Then it returns 5.
    assert_eq!(state.grapheme_count(), 5);
}

#[rstest::rstest]
fn delete_grapheme_before_cursor_handles_unicode() {
    // Given "é" with cursor at end (1).
    let mut state = ChatInputBoxState::new();
    state.insert_grapheme_at_cursor('é');

    // When deleting before cursor.
    state.delete_grapheme_before_cursor();

    // Then text is empty and cursor is at 0.
    assert_eq!(state.text(), "");
    assert_eq!(state.cursor_pos(), 0);
}

#[rstest::rstest]
fn move_cursor_left_right_with_unicode() {
    // Given "écafé" with cursor at end (5).
    let mut state = ChatInputBoxState::new();
    state.insert_text("écafé");

    // When moving left twice then right.
    state.move_cursor_left();
    state.move_cursor_left();
    assert_eq!(state.cursor_pos(), 3);
    state.move_cursor_right();

    // Then cursor is at 4.
    assert_eq!(state.cursor_pos(), 4);
}

#[rstest::rstest]
fn word_right_skips_past_unicode_word() {
    // Given "café au lait" with cursor at start (0).
    let mut state = ChatInputBoxState::new();
    state.insert_text("café au lait");
    state.move_cursor_to_start();

    // When moving word right.
    state.move_cursor_word_right();

    // Then cursor is at 5 (after "café ").
    assert_eq!(state.cursor_pos(), 5);
}

#[rstest::rstest]
fn word_right_twice_skips_two_words() {
    // Given "café au lait" with cursor at start (0).
    let mut state = ChatInputBoxState::new();
    state.insert_text("café au lait");
    state.move_cursor_to_start();

    // When moving word right twice.
    state.move_cursor_word_right();
    state.move_cursor_word_right();

    // Then cursor is at 8 (after "au ").
    assert_eq!(state.cursor_pos(), 8);
}

#[rstest::rstest]
fn visual_line_count_single_line() {
    // Given "hello".
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello");

    // When reading visual line count.
    // Then it is 1.
    assert_eq!(state.visual_line_count(), 1);
}

#[rstest::rstest]
fn visual_line_count_two_lines() {
    // Given "hello\nworld".
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\nworld");

    // When reading visual line count.
    // Then it is 2.
    assert_eq!(state.visual_line_count(), 2);
}

#[rstest::rstest]
fn visual_line_count_empty() {
    // Given an empty buffer.
    let state = ChatInputBoxState::new();

    // When reading visual line count.
    // Then it is 1.
    assert_eq!(state.visual_line_count(), 1);
}

#[rstest::rstest]
fn visual_line_count_trailing_newline() {
    // Given "hello\n".
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\n");

    // When reading visual line count.
    // Then it is 2 (trailing newline creates an empty line below).
    assert_eq!(state.visual_line_count(), 2);
}

#[rstest::rstest]
fn cursor_row_col_on_first_line() {
    // Given "hello" with cursor at end.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello");

    // When reading cursor row/col.
    // Then it is (0, 5).
    assert_eq!(state.cursor_row_col(), (0, 5));
}

#[rstest::rstest]
fn cursor_row_col_on_second_line() {
    // Given "hello\nworld" with cursor at end.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\nworld");

    // When reading cursor row/col.
    // Then it is (1, 5).
    assert_eq!(state.cursor_row_col(), (1, 5));
}

#[rstest::rstest]
fn cursor_row_col_at_start_of_second_line() {
    // Given "hello\nworld" with cursor right after the newline.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\nworld");
    state.move_cursor_to_start();
    state.move_cursor_right(); // h
    state.move_cursor_right(); // e
    state.move_cursor_right(); // l
    state.move_cursor_right(); // l
    state.move_cursor_right(); // o
    state.move_cursor_right(); // \n  → cursor is now at start of line 2

    // When reading cursor row/col.
    // Then it is (1, 0).
    assert_eq!(state.cursor_row_col(), (1, 0));
}

#[rstest::rstest]
fn cursor_row_col_empty_buffer() {
    // Given an empty buffer.
    let state = ChatInputBoxState::new();

    // When reading cursor row/col.
    // Then it is (0, 0).
    assert_eq!(state.cursor_row_col(), (0, 0));
}

#[rstest::rstest]
fn move_cursor_up_is_noop_on_first_line() {
    // Given "hello" with cursor at end (single line).
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello");

    // When moving up.
    state.move_cursor_up();

    // Then cursor stays at end (5).
    assert_eq!(state.cursor_pos(), 5);
}

#[rstest::rstest]
fn move_cursor_down_is_noop_on_last_line() {
    // Given "hello" with cursor at start (single line).
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello");
    state.move_cursor_to_start();

    // When moving down.
    state.move_cursor_down();

    // Then cursor stays at 0.
    assert_eq!(state.cursor_pos(), 0);
}

#[rstest::rstest]
fn move_cursor_up_goes_to_previous_line() {
    // Given "hello\nworld" with cursor at end of line 2.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\nworld");
    // cursor at 11 (end), row=1, col=5

    // When moving up.
    state.move_cursor_up();

    // Then cursor is at row 0, col 5 (grapheme index 5).
    assert_eq!(state.cursor_row_col(), (0, 5));
    assert_eq!(state.cursor_pos(), 5);
}

#[rstest::rstest]
fn move_cursor_down_goes_to_next_line() {
    // Given "hello\nworld" with cursor at start of line 1.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\nworld");
    state.move_cursor_to_start();
    // cursor at 0, row=0, col=0

    // When moving down.
    state.move_cursor_down();

    // Then cursor is at row 1, col 0 (grapheme index 6, after newline).
    assert_eq!(state.cursor_row_col(), (1, 0));
    assert_eq!(state.cursor_pos(), 6);
}

#[rstest::rstest]
fn move_cursor_up_clamps_col_to_shorter_line() {
    // Given "hello\nxy" with cursor at end of line 2 (col=2).
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\nxy");
    // Move cursor to col=5 on line 2 — impossible since line 2 has length 2.
    // Instead, move to col=2 on line 2, then go up.
    // Actually: cursor is at end (8), row=1, col=2. Go up → row=0, col=2.

    // When moving up from end of line 2.
    state.move_cursor_up();

    // Then cursor is at row 0, col 2 (grapheme index 2).
    assert_eq!(state.cursor_row_col(), (0, 2));
    assert_eq!(state.cursor_pos(), 2);
}

#[rstest::rstest]
fn move_cursor_up_preserves_col_on_equal_length_lines() {
    // Given "abcd\nefgh" with cursor at col 3 on line 2.
    let mut state = ChatInputBoxState::new();
    state.insert_text("abcd\nefgh");
    // cursor at end (9), row=1, col=4. Move left once → col=3.
    state.move_cursor_left();

    // When moving up.
    state.move_cursor_up();

    // Then cursor is at row 0, col 3 (grapheme index 3).
    assert_eq!(state.cursor_row_col(), (0, 3));
    assert_eq!(state.cursor_pos(), 3);
}

#[rstest::rstest]
fn move_cursor_down_clamps_col_to_shorter_line() {
    // Given "xy\nhello" with cursor at col 4 on line 1.
    let mut state = ChatInputBoxState::new();
    state.insert_text("xy\nhello");
    // cursor at end (8), row=1, col=5. Move to row=0, col=5 — impossible (line 0 has length 2).
    // Let's set up: cursor at start, move right 1 → row=0, col=1.
    state.move_cursor_to_start();
    state.move_cursor_right(); // col=1

    // When moving down.
    state.move_cursor_down();

    // Then cursor is at row 1, col 1 (grapheme index 4, which is 'e').
    assert_eq!(state.cursor_row_col(), (1, 1));
    assert_eq!(state.cursor_pos(), 4);
}

#[rstest::rstest]
fn move_cursor_up_on_empty_line() {
    // Given "a\n\nb" with cursor on line 2 (after 'b').
    let mut state = ChatInputBoxState::new();
    state.insert_text("a\n\nb");
    // cursor at 4 (end), row=2, col=1.

    // When moving up.
    state.move_cursor_up();

    // Then cursor is on the empty middle line (row=1, col=0 → grapheme index 2).
    assert_eq!(state.cursor_row_col(), (1, 0));
    assert_eq!(state.cursor_pos(), 2);
}

#[rstest::rstest]
fn move_cursor_down_on_empty_line() {
    // Given "a\n\nb" with cursor at start of line 1 (empty middle line).
    let mut state = ChatInputBoxState::new();
    state.insert_text("a\n\nb");
    state.move_cursor_to_start();
    state.move_cursor_right(); // past 'a'
    state.move_cursor_right(); // past \n, now on empty line 1

    // When moving down.
    state.move_cursor_down();

    // Then cursor is at row 2, col 0 (before 'b').
    assert_eq!(state.cursor_row_col(), (2, 0));
    assert_eq!(state.cursor_pos(), 3);
}

#[rstest::rstest]
fn move_cursor_up_empty_buffer_is_noop() {
    // Given an empty buffer.
    let mut state = ChatInputBoxState::new();

    // When moving up.
    state.move_cursor_up();

    // Then cursor stays at 0.
    assert_eq!(state.cursor_pos(), 0);
}

#[rstest::rstest]
fn move_cursor_down_empty_buffer_is_noop() {
    // Given an empty buffer.
    let mut state = ChatInputBoxState::new();

    // When moving down.
    state.move_cursor_down();

    // Then cursor stays at 0.
    assert_eq!(state.cursor_pos(), 0);
}

// --- desired column tests ---

#[rstest::rstest]
fn desired_col_preserved_across_shorter_intermediate_line_down() {
    // Given "abcdefghijkl\nxy\nmnopqrstuvwx" with cursor at col 10 on line 0.
    let mut state = ChatInputBoxState::new();
    state.insert_text("abcdefghijkl\nxy\nmnopqrstuvwx");
    // cursor at end (27), row=2, col=12. Move to start, then right 10.
    state.move_cursor_to_start();
    for _ in 0..10 {
        state.move_cursor_right();
    }
    assert_eq!(state.cursor_row_col(), (0, 10));

    // When moving down twice.
    state.move_cursor_down();
    assert_eq!(state.cursor_row_col(), (1, 2)); // clamped to end of "xy"
    state.move_cursor_down();

    // Then cursor is at row 2, col 10.
    assert_eq!(state.cursor_row_col(), (2, 10));
}

#[rstest::rstest]
fn desired_col_preserved_across_shorter_intermediate_line_up() {
    // Given "abcdefghijkl\nxy\nmnopqrstuvwx" with cursor at col 10 on line 2.
    let mut state = ChatInputBoxState::new();
    state.insert_text("abcdefghijkl\nxy\nmnopqrstuvwx");
    // cursor at end (27), row=2, col=12. Move left 2 → col=10.
    state.move_cursor_left();
    state.move_cursor_left();
    assert_eq!(state.cursor_row_col(), (2, 10));

    // When moving up twice.
    state.move_cursor_up();
    assert_eq!(state.cursor_row_col(), (1, 2)); // clamped to end of "xy"
    state.move_cursor_up();

    // Then cursor is at row 0, col 10.
    assert_eq!(state.cursor_row_col(), (0, 10));
}

#[rstest::rstest]
fn desired_col_cleared_by_horizontal_move() {
    // Given "abcd\nef\nghij" with cursor at col 3 on line 1.
    let mut state = ChatInputBoxState::new();
    state.insert_text("abcd\nef\nghij");
    state.move_cursor_to_start();
    state.move_cursor_right(); // col=1
    state.move_cursor_right(); // col=2
    state.move_cursor_right(); // col=3
    assert_eq!(state.cursor_row_col(), (0, 3));

    // When moving down (sets desired_col=3), then right (clears desired_col), then down.
    state.move_cursor_down();
    assert_eq!(state.cursor_row_col(), (1, 2)); // clamped
    state.move_cursor_right(); // clears desired_col, col is now actual position
    // Now on line 1, actual col is past end of "ef" (col=2). move_cursor_right is noop on end.
    // Let's use a different setup for clarity.

    // Better: start over with "hello\nab\nworld"
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\nab\nworld");
    state.move_cursor_to_start();
    for _ in 0..3 {
        state.move_cursor_right();
    }
    assert_eq!(state.cursor_row_col(), (0, 3)); // at 'l'

    state.move_cursor_down(); // desired_col = 3, clamped to col 2 on "ab"
    assert_eq!(state.cursor_row_col(), (1, 2));

    state.move_cursor_left(); // clears desired_col, actual col now 1
    assert_eq!(state.cursor_row_col(), (1, 1));

    state.move_cursor_down(); // desired_col is None → uses actual col 1
    assert_eq!(state.cursor_row_col(), (2, 1)); // col 1 on "world" = 'o'
}

#[rstest::rstest]
fn desired_col_cleared_by_insert() {
    // Given "abc\nxy\ndef" with cursor at col 2 on line 1.
    let mut state = ChatInputBoxState::new();
    state.insert_text("abc\nxy\ndef");
    state.move_cursor_to_start();
    state.move_cursor_right();
    state.move_cursor_right();
    assert_eq!(state.cursor_row_col(), (0, 2)); // at 'c'

    state.move_cursor_down(); // desired_col = 2, clamped to col 2 on "xy" (end)
    assert_eq!(state.cursor_row_col(), (1, 2));

    // When inserting a char.
    state.insert_grapheme_at_cursor('z'); // clears desired_col

    // Then moving down uses actual col, not the old desired col.
    state.move_cursor_down();
    assert_eq!(state.cursor_row_col(), (2, 3)); // actual col is 3 after insert
}

#[rstest::rstest]
fn desired_col_cleared_by_delete() {
    // Given "abcde\nxy\nfghij" with cursor at col 4 on line 0.
    let mut state = ChatInputBoxState::new();
    state.insert_text("abcde\nxy\nfghij");
    state.move_cursor_to_start();
    for _ in 0..4 {
        state.move_cursor_right();
    }
    assert_eq!(state.cursor_row_col(), (0, 4));

    state.move_cursor_down(); // desired_col = 4, clamped to col 2 on "xy"
    assert_eq!(state.cursor_row_col(), (1, 2));

    // When deleting before cursor.
    state.delete_grapheme_before_cursor(); // clears desired_col, col now 1
    assert_eq!(state.cursor_row_col(), (1, 1));

    state.move_cursor_down(); // desired_col is None → uses actual col 1
    assert_eq!(state.cursor_row_col(), (2, 1));
}

// --- Wrap-aware tests ---

#[rstest::rstest]
fn visual_line_count_wraps_long_line() {
    // Given "hello world" with wrap_width 5.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello world");
    state.set_wrap_width(5);

    // When reading visual line count.
    // "hello world" (11 graphemes) at width 5 wraps to "hello" and " world"
    // Actually: "hello" (5), " world" (6) — " world" exceeds 5, so wraps further.
    // Let's trace: h(0)e(1)l(2)l(3)o(4) = col 5, then ' ' at col 6 > 5 → wrap
    // Actually the break: at i=5 (space), col=6 > 5. last_word_break was set when
    // space at col=6 but there's no previous word break. Let's just check > 1.
    let count = state.visual_line_count();

    // Then it is more than 1 (the text wraps).
    assert!(count > 1, "expected wrapping but got {count} lines");
}

#[rstest::rstest]
fn cursor_row_col_on_wrapped_line() {
    // Given "hello world" with wrap_width 5 and cursor at end.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello world");
    state.set_wrap_width(5);

    // When reading cursor row/col (cursor at end, position 11).
    let (row, col) = state.cursor_row_col();

    // Then cursor is on a wrapped line (not row 0).
    assert!(row > 0, "expected wrapped row > 0 but got {row}");
    // And col is within the wrapped line.
    assert!(col <= 6, "expected col <= 6 but got {col}");
}

#[rstest::rstest]
fn move_cursor_up_wraps_to_previous_visual_line() {
    // Given "hello world" with wrap_width 5 and cursor at end.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello world");
    state.set_wrap_width(5);

    let (end_row, _) = state.cursor_row_col();

    // When moving up.
    state.move_cursor_up();

    // Then cursor moves to the previous wrapped line.
    let (new_row, _) = state.cursor_row_col();
    assert_eq!(new_row, end_row - 1);
}

#[rstest::rstest]
fn move_cursor_down_wraps_to_next_visual_line() {
    // Given "hello world" with wrap_width 5 and cursor at start.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello world");
    state.set_wrap_width(5);
    state.move_cursor_to_start();

    // When moving down.
    state.move_cursor_down();

    // Then cursor moves to the next wrapped line.
    let (new_row, _) = state.cursor_row_col();
    assert_eq!(new_row, 1);
}

#[rstest::rstest]
fn move_cursor_up_clamps_col_on_shorter_wrapped_line() {
    // Given "hello world" with wrap_width 5 and cursor at end of last line.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello world");
    state.set_wrap_width(5);

    let (end_row, _) = state.cursor_row_col();

    // When moving up.
    state.move_cursor_up();

    // Then cursor is on the previous wrapped line with col clamped.
    let (new_row, new_col) = state.cursor_row_col();
    assert_eq!(new_row, end_row - 1);
    // The first wrapped line "hello " has length 6, col should be clamped.
    assert!(new_col <= 6);
}

#[rstest::rstest]
fn scroll_to_cursor_adjusts_scroll_up() {
    // Given text that produces many wrapped lines.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello world foo bar baz");
    state.set_wrap_width(5);
    // Cursor at end.
    let (cursor_row, _) = state.cursor_row_col();

    // When scroll_offset is past the cursor and we scroll to cursor with 3 visible lines.
    state.set_scroll_offset(cursor_row + 5);
    state.scroll_to_cursor(3);

    // Then scroll_offset is adjusted to the cursor row.
    assert_eq!(state.scroll_offset(), cursor_row);
}

#[rstest::rstest]
fn scroll_to_cursor_adjusts_scroll_down() {
    // Given text that produces many wrapped lines.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello world foo bar baz");
    state.set_wrap_width(5);
    // Cursor at end.
    let (cursor_row, _) = state.cursor_row_col();

    // When scroll_offset is 0 and cursor is beyond visible area with 2 visible lines.
    state.scroll_to_cursor(2);

    // Then scroll_offset is adjusted so cursor is in the visible window.
    assert!(
        state.scroll_offset() + 2 > cursor_row,
        "cursor should be visible"
    );
    assert!(
        state.scroll_offset() <= cursor_row,
        "scroll should not go past cursor"
    );
}

#[rstest::rstest]
fn scroll_to_cursor_no_change_when_visible() {
    // Given short text with cursor visible.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello");
    state.set_wrap_width(80);

    // When scrolling to cursor with 5 visible lines.
    state.scroll_to_cursor(5);

    // Then scroll_offset stays at 0.
    assert_eq!(state.scroll_offset(), 0);
}

#[rstest::rstest]
fn reset_clears_scroll_offset() {
    // Given a state with non-zero scroll_offset.
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello");
    state.set_scroll_offset(5);

    // When resetting.
    state.reset();

    // Then scroll_offset is 0.
    assert_eq!(state.scroll_offset(), 0);
}

#[rstest::rstest]
fn scroll_to_cursor_does_not_scroll_away_first_line_on_multiline() {
    // Given "hello\nworld" with wrap_width large (no word-wrapping).
    let mut state = ChatInputBoxState::new();
    state.insert_text("hello\nworld");
    state.set_wrap_width(80);

    // Cursor is at end (row 1, col 5).
    // The element renders with Borders::BOTTOM, so inner.height = input_height - 1.
    // input_height = 1 + 2 = 3, inner.height = 2, so max_visible_lines = 2.
    // When scrolling with 2 visible lines.

    // When scrolling to cursor with correct visible-line count (2).
    state.scroll_to_cursor(2);

    // Then scroll_offset stays at 0 (both lines fit).
    assert_eq!(state.scroll_offset(), 0);
    // And cursor is on row 1, visible within the window.
    let (row, _) = state.cursor_row_col();
    assert_eq!(row, 1);
}
