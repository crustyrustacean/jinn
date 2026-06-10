//! The user's in-progress message being composed in the input box.
//!
//! Both the text buffer and cursor position are private. All mutation goes through
//! semantic methods that keep the cursor in sync with the buffer content.

use unicode_segmentation::UnicodeSegmentation as _;

use super::autocomplete::AutocompleteState;
use super::autocomplete::AutocompleteTrigger;
use crate::feat::chat_input::AutocompleteMatch;

/// Submission mode for the chat input box.
///
/// Controls where `SubmitMessage` sends the text:
/// - `Queue`: enqueue a normal `UserMessage` on the turn queue.
/// - `Steer`: append a fragment to the in-memory steering buffer for mid-turn
///   injection, with a fall-through to `EnqueueUserMessage` when the session
///   is `Idle` (no live turn to steer into).
///
/// Mode is sticky across submissions and phase transitions; it does not persist
/// across app restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    /// Submissions go to the normal turn queue.
    Queue,
    /// Submissions route to the steering buffer (or fall back to queue when phase is Idle).
    #[default]
    Steer,
}

impl InputMode {
    /// Flip to the other mode.
    #[must_use]
    pub fn toggle(self) -> Self {
        match self {
            Self::Queue => Self::Steer,
            Self::Steer => Self::Queue,
        }
    }

    /// Short label for the input border badge (e.g. `"QUEUE"`, `"STEER"`).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Queue => "QUEUE",
            Self::Steer => "STEER",
        }
    }
}

/// The user's in-progress message being composed in the input box.
///
/// Both the text buffer and cursor position are private. All mutation goes through
/// semantic methods that keep the cursor in sync with the buffer content.
#[derive(Debug, Clone)]
pub struct ChatInputBoxState {
    /// Active submission mode (Queue vs Steer). Sticky, not persisted.
    input_mode: InputMode,
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
    /// Width available for text rendering (grapheme columns). Set during render.
    /// Defaults to `usize::MAX` (no wrapping) until first render.
    wrap_width: usize,
    /// Scroll offset: the first visual line index that is visible.
    scroll_offset: usize,
}

impl ChatInputBoxState {
    /// Create a new state with no text entered and cursor at position 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            input_mode: InputMode::default(),
            input_buffer: String::new(),
            cursor_pos: 0,
            desired_col: None,
            autocomplete: None,
            wrap_width: usize::MAX,
            scroll_offset: 0,
        }
    }

    /// Returns the current submission mode.
    #[must_use]
    pub fn input_mode(&self) -> InputMode {
        self.input_mode
    }

    /// Flip the submission mode Queue ↔ Steer.
    pub fn toggle_input_mode(&mut self) {
        self.input_mode = self.input_mode.toggle();
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

    /// Returns the number of visual lines (word-wrapped at `wrap_width`).
    #[must_use]
    pub fn visual_line_count(&self) -> usize {
        let lines = self.wrapped_lines();
        lines.len().max(1)
    }

    /// Returns the cursor's `(row, col)` position within the wrapped visual lines.
    ///
    /// Row is 0-indexed (visual line number after wrapping), col is the grapheme
    /// offset within that wrapped line.
    #[must_use]
    pub fn cursor_row_col(&self) -> (usize, usize) {
        let lines = self.wrapped_lines();
        self.cursor_row_col_wrapped(&lines)
    }

    /// Insert text at the current cursor position and advance the cursor by the
    /// number of graphemes in the text.
    ///
    /// Performs a single bulk insertion - O(n) overall instead of O(n²) when
    /// inserting many characters one at a time via
    /// [`insert_grapheme_at_cursor`](Self::insert_grapheme_at_cursor).
    /// Newlines are preserved in the buffer.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let byte_offset = self
            .input_buffer
            .grapheme_indices(true)
            .nth(self.cursor_pos)
            .map_or(self.input_buffer.len(), |(i, _)| i);
        self.input_buffer.insert_str(byte_offset, text);
        self.cursor_pos += text.graphemes(true).count();
        self.desired_col = None;
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
        self.scroll_offset = 0;
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

    /// Move the cursor up one visual line (after wrapping).
    ///
    /// Remembers the column across consecutive vertical moves, even when
    /// clamped by shorter lines. No-op when the cursor is on the first line.
    pub fn move_cursor_up(&mut self) {
        let lines = self.wrapped_lines();
        let (row, col) = self.cursor_row_col_wrapped(&lines);
        if row == 0 {
            return;
        }
        let target_col = *self.desired_col.get_or_insert(col);
        self.cursor_pos = self.grapheme_index_for_wrapped_row_col(&lines, row - 1, target_col);
    }

    /// Move the cursor down one visual line (after wrapping).
    ///
    /// Remembers the column across consecutive vertical moves, even when
    /// clamped by shorter lines. No-op when the cursor is on the last line.
    pub fn move_cursor_down(&mut self) {
        let lines = self.wrapped_lines();
        let (row, col) = self.cursor_row_col_wrapped(&lines);
        let last_row = lines.len().saturating_sub(1);
        if row >= last_row {
            return;
        }
        let target_col = *self.desired_col.get_or_insert(col);
        self.cursor_pos = self.grapheme_index_for_wrapped_row_col(&lines, row + 1, target_col);
    }

    /// Compute the grapheme index for a given `(visual_row, col)` position
    /// in the wrapped line array.
    ///
    /// Clamps `col` to the length of the target wrapped line.
    fn grapheme_index_for_wrapped_row_col(
        &self,
        lines: &[super::wrap::WrappedLine],
        target_row: usize,
        target_col: usize,
    ) -> usize {
        let Some(line) = lines.get(target_row) else {
            return self.cursor_pos; // out of bounds, no-op
        };
        let line_len = line.grapheme_end.saturating_sub(line.grapheme_start);
        let clamped_col = target_col.min(line_len);
        line.grapheme_start + clamped_col
    }

    // -----------------------------------------------------------------------
    // Wrap-aware methods
    // -----------------------------------------------------------------------

    /// Sets the wrap width (called during render).
    pub fn set_wrap_width(&mut self, width: usize) {
        self.wrap_width = width;
    }

    /// Returns the current scroll offset.
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Sets the scroll offset directly.
    ///
    /// Used for testing. Normally, use [`scroll_to_cursor`](Self::scroll_to_cursor)
    /// to adjust the offset.
    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
    }

    /// Adjusts scroll offset to ensure the cursor's visual row is visible
    /// within `max_visible_lines` rows.
    #[expect(
        clippy::else_if_without_else,
        reason = "no-op on fallthrough is intentional"
    )]
    pub fn scroll_to_cursor(&mut self, max_visible_lines: usize) {
        let lines = self.wrapped_lines();
        let (cursor_row, _) = self.cursor_row_col_wrapped(&lines);

        if cursor_row < self.scroll_offset {
            self.scroll_offset = cursor_row;
        } else if max_visible_lines > 0 && cursor_row >= self.scroll_offset + max_visible_lines {
            self.scroll_offset = cursor_row - max_visible_lines + 1;
        }
    }

    /// Computes wrapped lines for the current buffer and wrap_width.
    pub fn wrapped_lines(&self) -> Vec<super::wrap::WrappedLine> {
        super::wrap::wrap_text(&self.input_buffer, self.wrap_width)
    }

    /// Returns cursor (visual_row, col_within_wrapped_line) using pre-computed lines.
    fn cursor_row_col_wrapped(&self, lines: &[super::wrap::WrappedLine]) -> (usize, usize) {
        for (row, line) in lines.iter().enumerate() {
            if self.cursor_pos >= line.grapheme_start && self.cursor_pos < line.grapheme_end {
                let col = self.cursor_pos - line.grapheme_start;
                return (row, col);
            }
        }

        // Cursor is at the boundary between lines (on a '\n' or at end of last line).
        // Find the line whose grapheme_end is <= cursor_pos and closest to it.
        let mut best_row = 0;
        let mut best_start = 0;
        for (row, line) in lines.iter().enumerate() {
            if line.grapheme_end <= self.cursor_pos {
                best_row = row;
                best_start = line.grapheme_start;
            }
        }
        (best_row, self.cursor_pos.saturating_sub(best_start))
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

    /// Activates autocomplete at the given grapheme index (where `#` or `/` was typed).
    ///
    /// If `matches` is empty, `selected_index` is set to 0.
    /// If non-empty, `selected_index` defaults to the last entry (most relevant).
    pub fn activate_autocomplete(
        &mut self,
        token_start: usize,
        trigger: AutocompleteTrigger,
        matches: Vec<AutocompleteMatch>,
    ) {
        let selected_index = if matches.is_empty() {
            0
        } else {
            matches.len() - 1
        };
        self.autocomplete = Some(AutocompleteState {
            trigger,
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

    /// Returns the screen column of the autocomplete `#` trigger within its visual line.
    ///
    /// The column is a grapheme offset within the line that contains the `#`.
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

    /// Completes the autocomplete: replaces the trigger region with the completed text.
    ///
    /// For `Hash` trigger: replaces `#partial` with `#name`.
    /// For `Slash` trigger: replaces `/partial` with `/name`.
    ///
    /// The region replaced is `token_start..cursor_pos` (grapheme indices).
    /// After completion, the cursor lands after the completed text,
    /// and autocomplete remains active with updated filter.
    pub fn complete_autocomplete(&mut self, name: &str) {
        let Some(ac) = self.autocomplete.as_ref() else {
            return;
        };
        let token_start = ac.token_start;
        let prefix = match ac.trigger {
            AutocompleteTrigger::Hash => '#',
            AutocompleteTrigger::Slash => '/',
        };
        let replacement = format!("{prefix}{name}");
        self.replace_grapheme_range(token_start, self.cursor_pos, &replacement);
        // Cursor is now after the replacement text.
        // Autocomplete stays active - the filter is now the exact name.
    }

    /// Expands a double-`#` token: replaces `#name#` with the template body.
    ///
    /// The region replaced is `token_start..cursor_pos` (which includes `#name#`).
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
    use crate::feat::chat_input::AutocompleteMatch;

    // --- Cursor movement ---

    #[rstest::rstest]
    fn move_cursor_left_at_zero_is_noop() {
        // Given an empty buffer.
        let mut state = ChatInputBoxState::new();

        // When moving left.
        state.move_cursor_left();

        // Then cursor stays at 0.
        assert_eq!(state.cursor_pos(), 0);
    }

    #[rstest::rstest]
    fn move_cursor_left_decrements() {
        // Given "abc" with cursor at end.
        let mut state = ChatInputBoxState::new();
        state.insert_text("abc");

        // When moving left.
        state.move_cursor_left();

        // Then cursor is at 2.
        assert_eq!(state.cursor_pos(), 2);
    }

    #[rstest::rstest]
    fn move_cursor_right_at_end_is_noop() {
        // Given "abc" with cursor at end.
        let mut state = ChatInputBoxState::new();
        state.insert_text("abc");

        // When moving right.
        state.move_cursor_right();

        // Then cursor stays at 3.
        assert_eq!(state.cursor_pos(), 3);
    }

    #[rstest::rstest]
    fn move_cursor_right_increments() {
        // Given "abc" with cursor at start.
        let mut state = ChatInputBoxState::new();
        state.insert_text("abc");
        state.move_cursor_to_start();

        // When moving right.
        state.move_cursor_right();

        // Then cursor is at 1.
        assert_eq!(state.cursor_pos(), 1);
    }

    #[rstest::rstest]
    fn move_cursor_word_left_skips_word_and_whitespace() {
        // Given "hello world" with cursor at end.
        let mut state = ChatInputBoxState::new();
        state.insert_text("hello world");

        // When moving word left.
        state.move_cursor_word_left();

        // Then cursor is at start of "world".
        assert_eq!(state.cursor_pos(), 6);
    }

    #[rstest::rstest]
    fn move_cursor_word_left_at_word_boundary() {
        // Given "hello   world" with cursor at end.
        let mut state = ChatInputBoxState::new();
        state.insert_text("hello   world");

        // When moving word left once.
        state.move_cursor_word_left();
        // Cursor at start of "world".
        assert_eq!(state.cursor_pos(), 8);

        // When moving word left again.
        state.move_cursor_word_left();
        // Cursor at start of "hello".
        assert_eq!(state.cursor_pos(), 0);
    }

    #[rstest::rstest]
    fn move_cursor_word_right_skips_word_and_whitespace() {
        // Given "hello world" with cursor at start.
        let mut state = ChatInputBoxState::new();
        state.insert_text("hello world");
        state.move_cursor_to_start();

        // When moving word right.
        state.move_cursor_word_right();

        // Then cursor is at start of "world".
        assert_eq!(state.cursor_pos(), 6);
    }

    // --- replace_grapheme_range ---

    #[rstest::rstest]
    fn replace_grapheme_range_replaces_middle() {
        // Given "hello world" with cursor at end (grapheme 11).
        let mut state = ChatInputBoxState::new();
        state.insert_text("hello world");
        // Cursor is now at grapheme 11 (after "world").

        // Activate autocomplete with token_start at grapheme 6 (start of "world").
        // complete_autocomplete will call replace_grapheme_range(6, 11, "#there").
        state.activate_autocomplete(
            6,
            AutocompleteTrigger::Hash,
            vec![AutocompleteMatch {
                name: "there".to_owned(),
                description: String::new(),
            }],
        );

        // When completing autocomplete with "there".
        state.complete_autocomplete("there");

        // Then graphemes 6..11 ("world") are replaced with "#there",
        // producing "hello #there".
        assert_eq!(state.text(), "hello #there");
        // Cursor is after the replacement: 6 (start) + 6 (graphemes in "#there").
        assert_eq!(state.cursor_pos(), 12);
    }

    // --- scroll_to_cursor ---

    #[rstest::rstest]
    fn scroll_to_cursor_when_cursor_above_offset() {
        // Given a buffer with many lines and scroll offset at 5.
        let mut state = ChatInputBoxState::new();
        state.set_wrap_width(80);
        for i in 0..20 {
            state.insert_text(&format!("line{i}"));
            if i < 19 {
                state.insert_grapheme_at_cursor('\n');
            }
        }
        state.set_scroll_offset(5);
        // Move cursor to row 3 (line3).
        state.move_cursor_to_start();
        for _ in 0..3 {
            state.move_cursor_down();
        }

        // When scrolling to cursor.
        state.scroll_to_cursor(10);

        // Then offset moves to the cursor's row.
        assert!(state.scroll_offset() < 5);
    }

    #[rstest::rstest]
    fn scroll_to_cursor_when_cursor_below_visible_range() {
        // Given a buffer with many lines.
        let mut state = ChatInputBoxState::new();
        for _ in 0..20 {
            state.insert_grapheme_at_cursor('\n');
        }
        state.set_wrap_width(80);
        state.set_scroll_offset(0);

        // Move cursor to near end.
        state.move_cursor_to_end();

        // When scrolling with 5 visible lines.
        state.scroll_to_cursor(5);

        // Then offset is adjusted so cursor is visible.
        assert!(state.scroll_offset() > 0);
    }

    // --- Autocomplete methods ---

    #[rstest::rstest]
    fn autocomplete_token_start_returns_none_when_inactive() {
        // Given a state without autocomplete.
        let state = ChatInputBoxState::new();

        // Then token_start is None.
        assert!(state.autocomplete_token_start().is_none());
    }

    #[rstest::rstest]
    fn autocomplete_token_start_returns_correct_index() {
        // Given a state with active autocomplete at position 5.
        let mut state = ChatInputBoxState::new();
        state.insert_text("hello#");
        state.activate_autocomplete(
            5,
            AutocompleteTrigger::Hash,
            vec![AutocompleteMatch {
                name: "test".to_owned(),
                description: String::new(),
            }],
        );

        // When reading token_start.
        // Then it returns 5 (not 0, not 1).
        let start = state.autocomplete_token_start();
        assert_eq!(start, Some(5));
        assert_ne!(start, Some(0));
        assert_ne!(start, Some(1));
    }

    #[rstest::rstest]
    fn autocomplete_token_screen_col_with_newlines() {
        // Given "hello\nworld#" with autocomplete at the #.
        let mut state = ChatInputBoxState::new();
        state.insert_text("hello\nworld#");
        state.activate_autocomplete(
            11, // grapheme index of #
            AutocompleteTrigger::Hash,
            vec![],
        );

        // When reading screen col.
        let col = state.autocomplete_token_screen_col();

        // Then col is 5 (0-indexed position within second line after reset).
        assert_eq!(col, Some(5));
    }

    #[rstest::rstest]
    fn autocomplete_token_screen_col_at_line_start() {
        // Given "hello\n#" with autocomplete at #.
        let mut state = ChatInputBoxState::new();
        state.insert_text("hello\n#");
        state.activate_autocomplete(
            6, // grapheme index of #
            AutocompleteTrigger::Hash,
            vec![],
        );

        // When reading screen col.
        let col = state.autocomplete_token_screen_col();

        // Then col is 0 (start of line after newline).
        assert_eq!(col, Some(0));
    }

    #[rstest::rstest]
    fn autocomplete_filter_returns_text_after_trigger() {
        // Given "#ab" with autocomplete at # and cursor after "ab".
        let mut state = ChatInputBoxState::new();
        state.insert_text("#ab");
        state.activate_autocomplete(0, AutocompleteTrigger::Hash, vec![]);

        // When reading filter.
        let filter = state.autocomplete_filter();

        // Then it is "ab".
        assert_eq!(filter, Some("ab".to_owned()));
    }

    #[rstest::rstest]
    fn autocomplete_filter_returns_empty_when_cursor_at_trigger() {
        // Given "#" with cursor right after #.
        let mut state = ChatInputBoxState::new();
        state.insert_text("#");
        state.activate_autocomplete(0, AutocompleteTrigger::Hash, vec![]);

        // When reading filter.
        let filter = state.autocomplete_filter();

        // Then it is empty.
        assert_eq!(filter, Some(String::new()));
    }

    #[rstest::rstest]
    fn autocomplete_move_down_changes_selection() {
        // Given autocomplete with 3 matches, selected at index 1.
        let mut state = ChatInputBoxState::new();
        state.insert_text("#");
        let matches = vec![
            AutocompleteMatch {
                name: "a".to_owned(),
                description: String::new(),
            },
            AutocompleteMatch {
                name: "b".to_owned(),
                description: String::new(),
            },
            AutocompleteMatch {
                name: "c".to_owned(),
                description: String::new(),
            },
        ];
        state.activate_autocomplete(0, AutocompleteTrigger::Hash, matches.clone());
        // Default selection is last (index 2).

        // Move up to index 1.
        state.autocomplete_move_up();
        assert_eq!(state.autocomplete().as_ref().unwrap().selected_index(), 1);

        // When moving down.
        state.autocomplete_move_down();

        // Then selection moves to 2.
        assert_eq!(state.autocomplete().as_ref().unwrap().selected_index(), 2);
    }

    #[rstest::rstest]
    fn expand_autocomplete_replaces_and_deactivates() {
        // Given "#greet#" with autocomplete active.
        let mut state = ChatInputBoxState::new();
        state.insert_text("#greet#");
        state.activate_autocomplete(0, AutocompleteTrigger::Hash, vec![]);

        // When expanding with body text.
        state.expand_autocomplete("Hello, world!");

        // Then the buffer is replaced and autocomplete is deactivated.
        assert_eq!(state.text(), "Hello, world!");
        assert!(state.autocomplete().is_none());
    }

    #[rstest::rstest]
    fn update_autocomplete_matches_updates_list() {
        // Given autocomplete with 2 matches.
        let mut state = ChatInputBoxState::new();
        state.insert_text("#");
        let initial = vec![AutocompleteMatch {
            name: "a".to_owned(),
            description: String::new(),
        }];
        state.activate_autocomplete(0, AutocompleteTrigger::Hash, initial);

        // When updating matches.
        let updated = vec![
            AutocompleteMatch {
                name: "b".to_owned(),
                description: String::new(),
            },
            AutocompleteMatch {
                name: "c".to_owned(),
                description: String::new(),
            },
        ];
        state.update_autocomplete_matches(updated);

        // Then the matches list is updated.
        let ac = state.autocomplete().as_ref().unwrap();
        assert_eq!(ac.matches().len(), 2);
        assert_eq!(ac.matches()[0].name, "b");
    }

    #[rstest::rstest]
    fn autocomplete_mut_returns_mutable_reference() {
        // Given a state with active autocomplete.
        let mut state = ChatInputBoxState::new();
        state.insert_text("#");
        state.activate_autocomplete(
            0,
            AutocompleteTrigger::Hash,
            vec![AutocompleteMatch {
                name: "a".to_owned(),
                description: String::new(),
            }],
        );

        // When getting mutable reference.
        let ac = state.autocomplete_mut();

        // Then it is Some and can be modified.
        assert!(ac.is_some());
        // The returned reference is to the actual autocomplete state, not a leaked box.
        // Verify by checking the matches.
        assert_eq!(ac.as_ref().unwrap().matches().len(), 1);
    }
}
