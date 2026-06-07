//! A single-line text input with grapheme-aware cursor editing.
//!
//! Shared by all single-line popup inputs ([`crate::feat::session_lifecycle`]
//! arg input, [`crate::feat::rename_session_input`], and the CWD input popup).
//!
//! The cursor is a **byte** offset into [`LineInput::input`], always landed on a
//! grapheme boundary. This matches the contract previously duplicated across
//! the per-feature text-edit handlers.

use unicode_segmentation::UnicodeSegmentation;

/// A single-line editable text field with a byte-offset cursor.
///
/// `cursor_pos` is a byte offset into `input` and is always positioned on a
/// grapheme boundary (the editing methods guarantee this). All grapheme-aware
/// operations use UAX #29 extended grapheme clusters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    /// The raw text.
    pub input: String,
    /// Byte offset of the cursor (always on a grapheme boundary).
    pub cursor_pos: usize,
}

impl LineInput {
    /// Creates an empty [`LineInput`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
        }
    }

    /// Replaces the text and moves the cursor to the end.
    pub fn set(&mut self, input: String) {
        self.cursor_pos = input.len();
        self.input = input;
    }

    /// Inserts a character at the cursor, advancing the cursor by its UTF-8 byte length.
    pub fn insert_char(&mut self, ch: char) {
        self.input.insert(self.cursor_pos, ch);
        self.cursor_pos += ch.len_utf8();
    }

    /// Bulk-inserts `text` at the cursor, advancing the cursor by `text.len()` bytes.
    ///
    /// No-op when `text` is empty.
    pub fn paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.input.insert_str(self.cursor_pos, text);
        self.cursor_pos += text.len();
    }

    /// Deletes the grapheme immediately before the cursor.
    ///
    /// No-op when the cursor is at position 0.
    pub fn delete(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.input[..self.cursor_pos]
                .grapheme_indices(true)
                .next_back()
                .map(|(i, _)| i);
            if let Some(prev_idx) = prev {
                self.input.drain(prev_idx..self.cursor_pos);
                self.cursor_pos = prev_idx;
            }
        }
    }

    /// Deletes the grapheme at or after the cursor (forward delete).
    ///
    /// No-op when the cursor is at the end of the input.
    pub fn delete_forward(&mut self) {
        if self.cursor_pos < self.input.len() {
            let next_end = self.input[self.cursor_pos..]
                .grapheme_indices(true)
                .nth(1)
                .map_or(self.input.len(), |(i, _)| self.cursor_pos + i);
            self.input.drain(self.cursor_pos..next_end);
        }
    }

    /// Moves the cursor one grapheme to the left.
    ///
    /// No-op when the cursor is at position 0.
    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.input[..self.cursor_pos]
                .grapheme_indices(true)
                .next_back()
                .map(|(i, _)| i);
            if let Some(prev_idx) = prev {
                self.cursor_pos = prev_idx;
            }
        }
    }

    /// Moves the cursor one grapheme to the right.
    ///
    /// No-op when the cursor is at the end of the input.
    pub fn cursor_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            let next = self.input[self.cursor_pos..]
                .grapheme_indices(true)
                .nth(1)
                .map(|(i, _)| self.cursor_pos + i);
            match next {
                Some(next_idx) => self.cursor_pos = next_idx,
                None => self.cursor_pos = self.input.len(),
            }
        }
    }

    /// Returns the number of graphemes before the cursor.
    ///
    /// Convenience for render code that needs a display (column) cursor offset.
    #[must_use]
    pub fn graphemes_before_cursor(&self) -> usize {
        self.input[..self.cursor_pos].graphemes(true).count()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn new_is_empty() {
        // Given a new LineInput.
        let li = LineInput::new();

        // Then it is empty with cursor at 0.
        assert!(li.input.is_empty());
        assert_eq!(li.cursor_pos, 0);
    }

    #[rstest::rstest]
    fn set_replaces_and_moves_cursor_to_end() {
        // Given a LineInput with prior content.
        let mut li = LineInput::new();
        li.insert_char('a');

        // When setting new text.
        li.set("hello".to_owned());

        // Then the text is replaced and cursor is at the end.
        assert_eq!(li.input, "hello");
        assert_eq!(li.cursor_pos, 5);
    }

    #[rstest::rstest]
    fn insert_char_appends_at_end() {
        // Given an empty LineInput.
        let mut li = LineInput::new();

        // When inserting a char.
        li.insert_char('x');

        // Then it is appended and the cursor advances.
        assert_eq!(li.input, "x");
        assert_eq!(li.cursor_pos, 1);
    }

    #[rstest::rstest]
    fn insert_char_multibyte_advances_by_utf8_len() {
        // Given a LineInput with cursor mid-string.
        let mut li = LineInput::new();
        li.set("ab".to_owned()); // cursor at 2

        // Inserting 'é' (2 bytes) at the end.
        li.insert_char('é');

        // Then the cursor advanced by the byte length, not 1.
        assert_eq!(li.input, "abé");
        assert_eq!(li.cursor_pos, 4); // a,b,0xc3,0xa9
    }

    #[rstest::rstest]
    fn insert_char_at_cursor_position() {
        // Given a LineInput with cursor in the middle.
        let mut li = LineInput::new();
        li.set("hello".to_owned());
        li.cursor_pos = 2; // between 'l' and 'l'

        // When inserting 'X'.
        li.insert_char('X');

        // Then it is inserted at the cursor.
        assert_eq!(li.input, "heXllo");
        assert_eq!(li.cursor_pos, 3);
    }

    #[rstest::rstest]
    fn paste_inserts_and_advances() {
        // Given a LineInput with cursor in the middle.
        let mut li = LineInput::new();
        li.set("hello".to_owned());
        li.cursor_pos = 2;

        // When pasting "XY".
        li.paste("XY");

        // Then text is inserted and cursor advances by text.len().
        assert_eq!(li.input, "heXYllo");
        assert_eq!(li.cursor_pos, 4);
    }

    #[rstest::rstest]
    fn paste_noop_when_empty() {
        // Given a LineInput.
        let mut li = LineInput::new();
        li.set("hello".to_owned());
        li.cursor_pos = 2;

        // When pasting empty text.
        li.paste("");

        // Then nothing changes.
        assert_eq!(li.input, "hello");
        assert_eq!(li.cursor_pos, 2);
    }

    #[rstest::rstest]
    fn delete_removes_preceding_grapheme() {
        // Given a LineInput with cursor at the end.
        let mut li = LineInput::new();
        li.set("hello".to_owned());

        // When deleting.
        li.delete();

        // Then the last grapheme is removed and cursor moved back.
        assert_eq!(li.input, "hell");
        assert_eq!(li.cursor_pos, 4);
    }

    #[rstest::rstest]
    fn delete_multibyte_grapheme() {
        // Given a LineInput ending in a multibyte grapheme.
        let mut li = LineInput::new();
        li.set("abcé".to_owned()); // cursor at 5 (a,b,c,0xc3,0xa9)

        // When deleting.
        li.delete();

        // Then the whole 'é' grapheme (2 bytes) is removed, cursor at 3.
        assert_eq!(li.input, "abc");
        assert_eq!(li.cursor_pos, 3);
    }

    #[rstest::rstest]
    fn delete_noop_at_position_zero() {
        // Given a LineInput with cursor at 0.
        let mut li = LineInput::new();
        li.set("hello".to_owned());
        li.cursor_pos = 0;

        // When deleting.
        li.delete();

        // Then nothing changes (boundary: > vs >=).
        assert_eq!(li.input, "hello");
        assert_eq!(li.cursor_pos, 0);
    }

    #[rstest::rstest]
    fn delete_forward_removes_following_grapheme() {
        // Given a LineInput with cursor at position 1.
        let mut li = LineInput::new();
        li.set("hello".to_owned());
        li.cursor_pos = 1;

        // When forward deleting.
        li.delete_forward();

        // Then the following grapheme is removed, cursor stays.
        assert_eq!(li.input, "hllo");
        assert_eq!(li.cursor_pos, 1);
    }

    #[rstest::rstest]
    fn delete_forward_noop_at_end() {
        // Given a LineInput with cursor at the end.
        let mut li = LineInput::new();
        li.set("hello".to_owned());

        // When forward deleting.
        li.delete_forward();

        // Then nothing changes (boundary: < vs <=).
        assert_eq!(li.input, "hello");
        assert_eq!(li.cursor_pos, 5);
    }

    #[rstest::rstest]
    fn cursor_left_moves_back_one_grapheme() {
        // Given a LineInput with cursor at end.
        let mut li = LineInput::new();
        li.set("hi".to_owned());

        // When moving left.
        li.cursor_left();

        // Then cursor moved to 1.
        assert_eq!(li.cursor_pos, 1);
    }

    #[rstest::rstest]
    fn cursor_left_multibyte_grapheme_boundary() {
        // Given a LineInput "aé" with cursor at end (3 bytes).
        let mut li = LineInput::new();
        li.set("aé".to_owned()); // a, 0xc3, 0xa9 → len 3, cursor 3

        // When moving left once.
        li.cursor_left();

        // Then cursor lands at the 'é' grapheme start (byte 1), not byte 2.
        assert_eq!(li.cursor_pos, 1);
    }

    #[rstest::rstest]
    fn cursor_left_noop_at_zero() {
        // Given a LineInput with cursor at 0.
        let mut li = LineInput::new();
        li.set("hi".to_owned());
        li.cursor_pos = 0;

        // When moving left.
        li.cursor_left();

        // Then cursor stays at 0.
        assert_eq!(li.cursor_pos, 0);
    }

    #[rstest::rstest]
    fn cursor_right_moves_forward_one_grapheme() {
        // Given a LineInput with cursor at start.
        let mut li = LineInput::new();
        li.set("hi".to_owned());
        li.cursor_pos = 0;

        // When moving right.
        li.cursor_right();

        // Then cursor moved to 1.
        assert_eq!(li.cursor_pos, 1);
    }

    #[rstest::rstest]
    fn cursor_right_multibyte_grapheme_boundary() {
        // Given a LineInput "aé" with cursor at 1 (start of 'é').
        let mut li = LineInput::new();
        li.set("aé".to_owned());
        li.cursor_pos = 1;

        // When moving right once.
        li.cursor_right();

        // Then cursor advances to the end (byte 3), skipping the whole grapheme.
        assert_eq!(li.cursor_pos, 3);
    }

    #[rstest::rstest]
    fn cursor_right_noop_at_end() {
        // Given a LineInput with cursor at end.
        let mut li = LineInput::new();
        li.set("hi".to_owned());

        // When moving right.
        li.cursor_right();

        // Then cursor stays at end.
        assert_eq!(li.cursor_pos, 2);
    }

    #[rstest::rstest]
    fn graphemes_before_cursor_counts_display_columns() {
        // Given a LineInput "aéb" with cursor after the multibyte grapheme.
        let mut li = LineInput::new();
        li.set("aéb".to_owned()); // a, é(2 bytes), b → len 4
        li.cursor_pos = 3; // after 'é'

        // Then graphemes_before_cursor is 2 (a, é), not 3 bytes.
        assert_eq!(li.graphemes_before_cursor(), 2);
    }
}
