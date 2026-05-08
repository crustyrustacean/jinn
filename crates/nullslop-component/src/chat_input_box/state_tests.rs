use super::*;

#[test]
fn grapheme_count_returns_cluster_count() {
    // Given a state with "éNoël" inserted.
    let mut state = ChatInputBoxState::new();
    for ch in "éNoël".chars() {
        state.insert_grapheme_at_cursor(ch);
    }

    // When reading the grapheme count.
    // Then it returns 5.
    assert_eq!(state.grapheme_count(), 5);
}

#[test]
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

#[test]
fn move_cursor_left_right_with_unicode() {
    // Given "écafé" with cursor at end (5).
    let mut state = ChatInputBoxState::new();
    for ch in "écafé".chars() {
        state.insert_grapheme_at_cursor(ch);
    }

    // When moving left twice then right.
    state.move_cursor_left();
    state.move_cursor_left();
    assert_eq!(state.cursor_pos(), 3);
    state.move_cursor_right();

    // Then cursor is at 4.
    assert_eq!(state.cursor_pos(), 4);
}

#[test]
fn word_right_skips_past_unicode_word() {
    // Given "café au lait" with cursor at start (0).
    let mut state = ChatInputBoxState::new();
    for ch in "café au lait".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    state.move_cursor_to_start();

    // When moving word right.
    state.move_cursor_word_right();

    // Then cursor is at 5 (after "café ").
    assert_eq!(state.cursor_pos(), 5);
}

#[test]
fn word_right_twice_skips_two_words() {
    // Given "café au lait" with cursor at start (0).
    let mut state = ChatInputBoxState::new();
    for ch in "café au lait".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    state.move_cursor_to_start();

    // When moving word right twice.
    state.move_cursor_word_right();
    state.move_cursor_word_right();

    // Then cursor is at 8 (after "au ").
    assert_eq!(state.cursor_pos(), 8);
}

#[test]
fn visual_line_count_single_line() {
    // Given "hello".
    let mut state = ChatInputBoxState::new();
    for ch in "hello".chars() {
        state.insert_grapheme_at_cursor(ch);
    }

    // When reading visual line count.
    // Then it is 1.
    assert_eq!(state.visual_line_count(), 1);
}

#[test]
fn visual_line_count_two_lines() {
    // Given "hello\nworld".
    let mut state = ChatInputBoxState::new();
    for ch in "hello\nworld".chars() {
        state.insert_grapheme_at_cursor(ch);
    }

    // When reading visual line count.
    // Then it is 2.
    assert_eq!(state.visual_line_count(), 2);
}

#[test]
fn visual_line_count_empty() {
    // Given an empty buffer.
    let state = ChatInputBoxState::new();

    // When reading visual line count.
    // Then it is 1.
    assert_eq!(state.visual_line_count(), 1);
}

#[test]
fn visual_line_count_trailing_newline() {
    // Given "hello\n".
    let mut state = ChatInputBoxState::new();
    for ch in "hello\n".chars() {
        state.insert_grapheme_at_cursor(ch);
    }

    // When reading visual line count.
    // Then it is 2 (trailing newline creates an empty line below).
    assert_eq!(state.visual_line_count(), 2);
}

#[test]
fn cursor_row_col_on_first_line() {
    // Given "hello" with cursor at end.
    let mut state = ChatInputBoxState::new();
    for ch in "hello".chars() {
        state.insert_grapheme_at_cursor(ch);
    }

    // When reading cursor row/col.
    // Then it is (0, 5).
    assert_eq!(state.cursor_row_col(), (0, 5));
}

#[test]
fn cursor_row_col_on_second_line() {
    // Given "hello\nworld" with cursor at end.
    let mut state = ChatInputBoxState::new();
    for ch in "hello\nworld".chars() {
        state.insert_grapheme_at_cursor(ch);
    }

    // When reading cursor row/col.
    // Then it is (1, 5).
    assert_eq!(state.cursor_row_col(), (1, 5));
}

#[test]
fn cursor_row_col_at_start_of_second_line() {
    // Given "hello\nworld" with cursor right after the newline.
    let mut state = ChatInputBoxState::new();
    for ch in "hello\nworld".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
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

#[test]
fn cursor_row_col_empty_buffer() {
    // Given an empty buffer.
    let state = ChatInputBoxState::new();

    // When reading cursor row/col.
    // Then it is (0, 0).
    assert_eq!(state.cursor_row_col(), (0, 0));
}

#[test]
fn move_cursor_up_is_noop_on_first_line() {
    // Given "hello" with cursor at end (single line).
    let mut state = ChatInputBoxState::new();
    for ch in "hello".chars() {
        state.insert_grapheme_at_cursor(ch);
    }

    // When moving up.
    state.move_cursor_up();

    // Then cursor stays at end (5).
    assert_eq!(state.cursor_pos(), 5);
}

#[test]
fn move_cursor_down_is_noop_on_last_line() {
    // Given "hello" with cursor at start (single line).
    let mut state = ChatInputBoxState::new();
    for ch in "hello".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    state.move_cursor_to_start();

    // When moving down.
    state.move_cursor_down();

    // Then cursor stays at 0.
    assert_eq!(state.cursor_pos(), 0);
}

#[test]
fn move_cursor_up_goes_to_previous_line() {
    // Given "hello\nworld" with cursor at end of line 2.
    let mut state = ChatInputBoxState::new();
    for ch in "hello\nworld".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    // cursor at 11 (end), row=1, col=5

    // When moving up.
    state.move_cursor_up();

    // Then cursor is at row 0, col 5 (grapheme index 5).
    assert_eq!(state.cursor_row_col(), (0, 5));
    assert_eq!(state.cursor_pos(), 5);
}

#[test]
fn move_cursor_down_goes_to_next_line() {
    // Given "hello\nworld" with cursor at start of line 1.
    let mut state = ChatInputBoxState::new();
    for ch in "hello\nworld".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    state.move_cursor_to_start();
    // cursor at 0, row=0, col=0

    // When moving down.
    state.move_cursor_down();

    // Then cursor is at row 1, col 0 (grapheme index 6, after newline).
    assert_eq!(state.cursor_row_col(), (1, 0));
    assert_eq!(state.cursor_pos(), 6);
}

#[test]
fn move_cursor_up_clamps_col_to_shorter_line() {
    // Given "hello\nxy" with cursor at end of line 2 (col=2).
    let mut state = ChatInputBoxState::new();
    for ch in "hello\nxy".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    // Move cursor to col=5 on line 2 — impossible since line 2 has length 2.
    // Instead, move to col=2 on line 2, then go up.
    // Actually: cursor is at end (8), row=1, col=2. Go up → row=0, col=2.

    // When moving up from end of line 2.
    state.move_cursor_up();

    // Then cursor is at row 0, col 2 (grapheme index 2).
    assert_eq!(state.cursor_row_col(), (0, 2));
    assert_eq!(state.cursor_pos(), 2);
}

#[test]
fn move_cursor_up_preserves_col_on_equal_length_lines() {
    // Given "abcd\nefgh" with cursor at col 3 on line 2.
    let mut state = ChatInputBoxState::new();
    for ch in "abcd\nefgh".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    // cursor at end (9), row=1, col=4. Move left once → col=3.
    state.move_cursor_left();

    // When moving up.
    state.move_cursor_up();

    // Then cursor is at row 0, col 3 (grapheme index 3).
    assert_eq!(state.cursor_row_col(), (0, 3));
    assert_eq!(state.cursor_pos(), 3);
}

#[test]
fn move_cursor_down_clamps_col_to_shorter_line() {
    // Given "xy\nhello" with cursor at col 4 on line 1.
    let mut state = ChatInputBoxState::new();
    for ch in "xy\nhello".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
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

#[test]
fn move_cursor_up_on_empty_line() {
    // Given "a\n\nb" with cursor on line 2 (after 'b').
    let mut state = ChatInputBoxState::new();
    for ch in "a\n\nb".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    // cursor at 4 (end), row=2, col=1.

    // When moving up.
    state.move_cursor_up();

    // Then cursor is on the empty middle line (row=1, col=0 → grapheme index 2).
    assert_eq!(state.cursor_row_col(), (1, 0));
    assert_eq!(state.cursor_pos(), 2);
}

#[test]
fn move_cursor_down_on_empty_line() {
    // Given "a\n\nb" with cursor at start of line 1 (empty middle line).
    let mut state = ChatInputBoxState::new();
    for ch in "a\n\nb".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
    state.move_cursor_to_start();
    state.move_cursor_right(); // past 'a'
    state.move_cursor_right(); // past \n, now on empty line 1

    // When moving down.
    state.move_cursor_down();

    // Then cursor is at row 2, col 0 (before 'b').
    assert_eq!(state.cursor_row_col(), (2, 0));
    assert_eq!(state.cursor_pos(), 3);
}

#[test]
fn move_cursor_up_empty_buffer_is_noop() {
    // Given an empty buffer.
    let mut state = ChatInputBoxState::new();

    // When moving up.
    state.move_cursor_up();

    // Then cursor stays at 0.
    assert_eq!(state.cursor_pos(), 0);
}

#[test]
fn move_cursor_down_empty_buffer_is_noop() {
    // Given an empty buffer.
    let mut state = ChatInputBoxState::new();

    // When moving down.
    state.move_cursor_down();

    // Then cursor stays at 0.
    assert_eq!(state.cursor_pos(), 0);
}

// --- desired column tests ---

#[test]
fn desired_col_preserved_across_shorter_intermediate_line_down() {
    // Given "abcdefghijkl\nxy\nmnopqrstuvwx" with cursor at col 10 on line 0.
    let mut state = ChatInputBoxState::new();
    for ch in "abcdefghijkl\nxy\nmnopqrstuvwx".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
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

#[test]
fn desired_col_preserved_across_shorter_intermediate_line_up() {
    // Given "abcdefghijkl\nxy\nmnopqrstuvwx" with cursor at col 10 on line 2.
    let mut state = ChatInputBoxState::new();
    for ch in "abcdefghijkl\nxy\nmnopqrstuvwx".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
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

#[test]
fn desired_col_cleared_by_horizontal_move() {
    // Given "abcd\nef\nghij" with cursor at col 3 on line 1.
    let mut state = ChatInputBoxState::new();
    for ch in "abcd\nef\nghij".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
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
    for ch in "hello\nab\nworld".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
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

#[test]
fn desired_col_cleared_by_insert() {
    // Given "abc\nxy\ndef" with cursor at col 2 on line 1.
    let mut state = ChatInputBoxState::new();
    for ch in "abc\nxy\ndef".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
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

#[test]
fn desired_col_cleared_by_delete() {
    // Given "abcde\nxy\nfghij" with cursor at col 4 on line 0.
    let mut state = ChatInputBoxState::new();
    for ch in "abcde\nxy\nfghij".chars() {
        state.insert_grapheme_at_cursor(ch);
    }
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
