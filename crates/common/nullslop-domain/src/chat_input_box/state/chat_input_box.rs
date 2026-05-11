//! The user's in-progress message being composed in the input box.
//!
//! Both the text buffer and cursor position are private. All mutation goes through
//! semantic methods that keep the cursor in sync with the buffer content.

use unicode_segmentation::UnicodeSegmentation as _;

use super::autocomplete::AutocompleteState;
use crate::chat_input_box::AutocompleteMatch;

/// The user's in-progress message being composed in the input box.
///
/// Both the text buffer and cursor position are private. All mutation goes through
/// semantic methods that keep the cursor in sync with the buffer content.
#[derive(Debug)]
pub struct ChatInputBoxState {
    /// The text the user has typed so far.
    input_buffer: String,
    /// Cursor position as a grapheme-cluster index (0 = before first grapheme).
    cursor_pos: usize,
    /// The column remembered across consecutive up/down movements.
    ///
    /// Set on the first vertical move, preserved across subsequent vertical moves
    /// (even when clamped by shorter lines). Cleared by any non-vertical operation.
    desired_col: Option<usize>,
    /// Active prompt-template autocomplete session, if any.
    autocomplete: Option<AutocompleteState>,
}

impl ChatInputBoxState {
    /// Create a new state with no text entered and cursor at position 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            input_buffer: String::new(),
            cursor_pos: 0,
            desired_col: None,
            autocomplete: None,
        }
    }

    /// Returns a reference to the current input text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.input_buffer
    }

    /// Returns whether the input buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.input_buffer.is_empty()
    }

    /// Returns the current cursor position as a grapheme index.
    #[must_use]
    pub fn cursor_pos(&self) -> usize {
        self.cursor_pos
    }

    /// Returns the total number of grapheme clusters in the buffer.
    #[must_use]
    pub fn grapheme_count(&self) -> usize {
        self.input_buffer.graphemes(true).count()
    }

    /// Replaces a range of graphemes (start..end) with new text.
    ///
    /// Sets cursor to `start + new_text_grapheme_count`.
    fn replace_grapheme_range(&mut self, start: usize, end: usize, new_text: &str) {
        let graphemes: Vec<(usize, &str)> = self.input_buffer.grapheme_indices(true).collect();

        let byte_start = graphemes
            .get(start)
            .map_or(self.input_buffer.len(), |(i, _)| *i);
        let byte_end = graphemes
            .get(end)
            .map_or(self.input_buffer.len(), |(i, _)| *i);

        // Drain the old range.
        self.input_buffer.drain(byte_start..byte_end);

        // Insert new text at the same byte position.
        self.input_buffer.insert_str(byte_start, new_text);

        // Recompute cursor: start index + graphemes in new text.
        let new_grapheme_count = new_text.graphemes(true).count();
        self.cursor_pos = start + new_grapheme_count;
        self.desired_col = None;
    }

    /// Returns the grapheme at the given index, if it exists.
    #[must_use]
    pub fn grapheme_at(&self, index: usize) -> Option<&str> {
        self.input_buffer.graphemes(true).nth(index)
    }

    /// Returns the number of visual lines (splits on `\n` graphemes + 1).
    #[must_use]
    pub fn visual_line_count(&self) -> usize {
        if self.input_buffer.is_empty() {
            return 1;
        }
        let newline_count = self
            .input_buffer
            .graphemes(true)
            .filter(|g| *g == "\n")
            .count();
        newline_count + 1
    }

    /// Returns the cursor's `(row, col)` position within the multi-line buffer.
    ///
    /// Row is 0-indexed (line number), col is the grapheme offset within that line.
    #[must_use]
    pub fn cursor_row_col(&self) -> (usize, usize) {
        let mut row = 0;
        let mut col = 0;

        for (i, g) in self.input_buffer.graphemes(true).enumerate() {
            if i == self.cursor_pos {
                break;
            }
            if g == "\n" {
                row += 1;
                col = 0;
            } else {
                col += 1;
            }
        }

        (row, col)
    }

    /// Insert text at the current cursor position and advance the cursor by the
    /// number of graphemes in the text.
    ///
    /// Convenience method that loops over characters and calls
    /// [`insert_grapheme_at_cursor`](Self::insert_grapheme_at_cursor) for each.
    pub fn insert_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.insert_grapheme_at_cursor(ch);
        }
    }

    /// Insert a character at the current cursor position and advance the cursor by 1.
    pub fn insert_grapheme_at_cursor(&mut self, ch: char) {
        let byte_offset = self
            .input_buffer
            .grapheme_indices(true)
            .nth(self.cursor_pos)
            .map_or(self.input_buffer.len(), |(i, _)| i);
        self.input_buffer.insert(byte_offset, ch);
        self.cursor_pos += 1;
        self.desired_col = None;
    }

    /// Delete the grapheme immediately before the cursor and move the cursor back by 1.
    ///
    /// No-op when the cursor is at position 0.
    #[expect(
        clippy::indexing_slicing,
        reason = "delete_idx is cursor_pos - 1 where cursor_pos > 0, and graphemes length equals grapheme count which is >= cursor_pos"
    )]
    pub fn delete_grapheme_before_cursor(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let graphemes: Vec<(usize, &str)> = self.input_buffer.grapheme_indices(true).collect();
        let delete_idx = self.cursor_pos - 1;
        let (start, g) = graphemes[delete_idx];
        let end = start + g.len();
        self.input_buffer.drain(start..end);
        self.cursor_pos -= 1;
        self.desired_col = None;
    }

    /// Clear the buffer and reset the cursor to position 0.
    pub fn reset(&mut self) {
        self.input_buffer.clear();
        self.cursor_pos = 0;
        self.desired_col = None;
    }

    /// Replace the entire buffer content and position cursor at the end.
    ///
    /// Used when loading content from an external editor.
    pub fn replace_all(&mut self, content: String) {
        self.input_buffer = content;
        self.cursor_pos = self.input_buffer.graphemes(true).count();
        self.desired_col = None;
    }

    /// Move the cursor one grapheme to the left.
    ///
    /// No-op when the cursor is already at position 0.
    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
        self.desired_col = None;
    }

    /// Move the cursor one grapheme to the right.
    ///
    /// No-op when the cursor is already at the end of the buffer.
    pub fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.grapheme_count() {
            self.cursor_pos += 1;
        }
        self.desired_col = None;
    }

    /// Move the cursor to the beginning of the buffer.
    pub fn move_cursor_to_start(&mut self) {
        self.cursor_pos = 0;
        self.desired_col = None;
    }

    /// Move the cursor to the end of the buffer.
    pub fn move_cursor_to_end(&mut self) {
        self.cursor_pos = self.grapheme_count();
        self.desired_col = None;
    }

    /// Delete the grapheme at the cursor position (forward delete).
    ///
    /// No-op when the cursor is at the end of the buffer.
    #[expect(
        clippy::indexing_slicing,
        reason = "cursor_pos < count is checked above, so index is in bounds"
    )]
    pub fn delete_grapheme_after_cursor(&mut self) {
        let count = self.grapheme_count();
        if self.cursor_pos >= count {
            return;
        }
        let graphemes: Vec<(usize, &str)> = self.input_buffer.grapheme_indices(true).collect();
        let (start, g) = graphemes[self.cursor_pos];
        let end = start + g.len();
        self.input_buffer.drain(start..end);
        self.desired_col = None;
    }

    /// Move the cursor one word to the left.
    ///
    /// A word boundary is a transition from whitespace to non-whitespace.
    /// Scans left from the current cursor position, skips any whitespace,
    /// then finds the start of the preceding word.
    /// No-op when the cursor is at position 0.
    #[expect(
        clippy::indexing_slicing,
        reason = "pos > 0 is checked before indexing pos - 1, guaranteed in bounds"
    )]
    pub fn move_cursor_word_left(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let graphemes: Vec<&str> = self.input_buffer.graphemes(true).collect();
        let mut pos = self.cursor_pos;

        // Skip whitespace moving left.
        while pos > 0 && graphemes[pos - 1].trim().is_empty() {
            pos -= 1;
        }
        // Skip non-whitespace moving left (the word itself).
        while pos > 0 && !graphemes[pos - 1].trim().is_empty() {
            pos -= 1;
        }
        self.cursor_pos = pos;
        self.desired_col = None;
    }

    /// Move the cursor one word to the right.
    ///
    /// A word boundary is a transition from non-whitespace to whitespace.
    /// Scans right from the current cursor position, skips any non-whitespace,
    /// then skips any whitespace to land at the start of the next word.
    /// No-op when the cursor is at the end of the buffer.
    #[expect(
        clippy::indexing_slicing,
        reason = "pos < count is checked before indexing pos, guaranteed in bounds"
    )]
    pub fn move_cursor_word_right(&mut self) {
        let count = self.grapheme_count();
        if self.cursor_pos >= count {
            return;
        }
        let graphemes: Vec<&str> = self.input_buffer.graphemes(true).collect();
        let mut pos = self.cursor_pos;

        // Skip non-whitespace moving right (the current word).
        while pos < count && !graphemes[pos].trim().is_empty() {
            pos += 1;
        }
        // Skip whitespace moving right.
        while pos < count && graphemes[pos].trim().is_empty() {
            pos += 1;
        }
        self.cursor_pos = pos;
        self.desired_col = None;
    }

    /// Move the cursor up one visual line.
    ///
    /// Remembers the column across consecutive vertical moves, even when
    /// clamped by shorter lines. No-op when the cursor is on the first line.
    pub fn move_cursor_up(&mut self) {
        let (row, col) = self.cursor_row_col();
        if row == 0 {
            return;
        }
        let target_col = *self.desired_col.get_or_insert(col);
        self.cursor_pos = self.grapheme_index_for_row_col(row - 1, target_col);
    }

    /// Move the cursor down one visual line.
    ///
    /// Remembers the column across consecutive vertical moves, even when
    /// clamped by shorter lines. No-op when the cursor is on the last line.
    pub fn move_cursor_down(&mut self) {
        let (row, col) = self.cursor_row_col();
        let last_row = self.visual_line_count() - 1;
        if row >= last_row {
            return;
        }
        let target_col = *self.desired_col.get_or_insert(col);
        self.cursor_pos = self.grapheme_index_for_row_col(row + 1, target_col);
    }

    /// Compute the grapheme index for a given `(row, col)` position.
    ///
    /// Clamps `col` to the length of the target row's line.
    fn grapheme_index_for_row_col(&self, target_row: usize, target_col: usize) -> usize {
        let mut row = 0;
        let mut col = 0;
        let mut idx = 0;

        for g in self.input_buffer.graphemes(true) {
            if row == target_row && col == target_col {
                return idx;
            }
            if g == "\n" {
                if row == target_row {
                    // We've reached end of target line; col was too far, clamp.
                    return idx;
                }
                row += 1;
                col = 0;
            } else {
                col += 1;
            }
            idx += 1;
        }

        // If we ran out of graphemes on the target row, return end-of-buffer.
        idx
    }

    // -----------------------------------------------------------------------
    // Autocomplete methods
    // -----------------------------------------------------------------------

    /// Returns a reference to the active autocomplete state, if any.
    #[must_use]
    pub fn autocomplete(&self) -> &Option<AutocompleteState> {
        &self.autocomplete
    }

    /// Returns a mutable reference to the active autocomplete state.
    #[must_use]
    pub fn autocomplete_mut(&mut self) -> &mut Option<AutocompleteState> {
        &mut self.autocomplete
    }

    /// Deactivates autocomplete (dismisses the popup).
    pub fn deactivate_autocomplete(&mut self) {
        self.autocomplete = None;
    }

    /// Activates autocomplete at the given grapheme index (where `$` was typed).
    ///
    /// If `matches` is empty, `selected_index` is set to 0.
    /// If non-empty, `selected_index` defaults to the last entry (most relevant).
    pub fn activate_autocomplete(&mut self, token_start: usize, matches: Vec<AutocompleteMatch>) {
        let selected_index = if matches.is_empty() {
            0
        } else {
            matches.len() - 1
        };
        self.autocomplete = Some(AutocompleteState {
            token_start,
            selected_index,
            matches,
        });
    }

    /// Updates the match list in the active autocomplete state, clamping selection.
    pub fn update_autocomplete_matches(&mut self, matches: Vec<AutocompleteMatch>) {
        if let Some(ac) = &mut self.autocomplete {
            ac.set_matches(matches);
        }
    }

    /// Returns the current filter text derived from the buffer content.
    ///
    /// Extracts graphemes from `token_start + 1` to `cursor_pos`.
    /// Returns `None` if autocomplete is not active.
    #[must_use]
    pub fn autocomplete_filter(&self) -> Option<String> {
        let ac = self.autocomplete.as_ref()?;
        let start = ac.token_start + 1;
        let end = self.cursor_pos;
        if start >= end {
            return Some(String::new());
        }
        let filter: String = self
            .input_buffer
            .graphemes(true)
            .enumerate()
            .skip_while(|(i, _)| *i < start)
            .take_while(|(i, _)| *i < end)
            .map(|(_, g)| g)
            .collect();
        Some(filter)
    }

    /// Returns the currently selected autocomplete match.
    #[must_use]
    pub fn autocomplete_selected(&self) -> Option<&AutocompleteMatch> {
        self.autocomplete.as_ref()?.selected_match()
    }

    /// Moves the autocomplete selection up (toward less relevant).
    pub fn autocomplete_move_up(&mut self) {
        if let Some(ac) = &mut self.autocomplete {
            ac.move_up();
        }
    }

    /// Moves the autocomplete selection down (toward more relevant).
    pub fn autocomplete_move_down(&mut self) {
        if let Some(ac) = &mut self.autocomplete {
            ac.move_down();
        }
    }

    /// Returns the `token_start` grapheme index if autocomplete is active.
    #[must_use]
    pub fn autocomplete_token_start(&self) -> Option<usize> {
        self.autocomplete.as_ref().map(|ac| ac.token_start)
    }

    /// Returns the screen column of the autocomplete `$` trigger within its visual line.
    ///
    /// The column is a grapheme offset within the line that contains the `$`.
    /// Returns `None` if autocomplete is not active.
    #[must_use]
    pub fn autocomplete_token_screen_col(&self) -> Option<usize> {
        let ac = self.autocomplete.as_ref()?;
        let token_start = ac.token_start();
        let mut col = 0;
        for (i, g) in self.input_buffer.graphemes(true).enumerate() {
            if i == token_start {
                return Some(col);
            }
            if g == "\n" {
                col = 0;
            } else {
                col += 1;
            }
        }
        Some(col) // token_start at end of buffer
    }

    /// Completes the autocomplete: replaces the `$partial` region with `$name`.
    ///
    /// The region replaced is `token_start..cursor_pos` (grapheme indices).
    /// After completion, the cursor lands after the completed text,
    /// and autocomplete remains active with updated filter.
    pub fn complete_autocomplete(&mut self, name: &str) {
        let Some(ac) = self.autocomplete.as_ref() else {
            return;
        };
        let token_start = ac.token_start;
        let replacement = format!("${name}");
        self.replace_grapheme_range(token_start, self.cursor_pos, &replacement);
        // Cursor is now after the replacement text.
        // Autocomplete stays active — the filter is now the exact name.
    }

    /// Expands a double-`$` token: replaces `$name$` with the template body.
    ///
    /// The region replaced is `token_start..cursor_pos` (which includes `$name$`).
    /// After expansion, autocomplete is deactivated and the cursor lands after
    /// the body text.
    pub fn expand_autocomplete(&mut self, body: &str) {
        let Some(ac) = self.autocomplete.as_ref() else {
            return;
        };
        let token_start = ac.token_start;
        self.replace_grapheme_range(token_start, self.cursor_pos, body);
        self.autocomplete = None;
    }
}

impl Default for ChatInputBoxState {
    fn default() -> Self {
        Self::new()
    }
}
