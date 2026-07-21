//! Dynamic dispatch trait for picker navigation operations.
//!
//! Provides a trait object interface so callers can operate on any active picker
//! without knowing its concrete `SelectionState<T>` type parameter.

/// Trait for picker navigation operations.
///
/// Implemented automatically for all `SelectionState<T>` types.
/// Use as `&mut dyn PickerOps` for dynamic dispatch over picker kinds.
pub trait PickerOps {
    /// Appends a character to the filter at the cursor position.
    fn insert_char(&mut self, ch: char);

    /// Bulk inserts text into the filter at the cursor position.
    ///
    /// Newlines are stripped - the picker filter is a single line.
    fn insert_text(&mut self, text: &str);

    /// Removes the character before the cursor.
    fn backspace(&mut self);

    /// Moves the selection up by one item.
    fn move_up(&mut self, max_visible: usize);

    /// Moves the selection down by one item.
    fn move_down(&mut self, max_visible: usize);

    /// Moves the selection up by half of `max_visible`, clamped at 0, then
    /// keeps it within the scroll window.
    fn page_up(&mut self, max_visible: usize);

    /// Moves the selection down by half of `max_visible`, clamped at the
    /// end of the list, then keeps it within the scroll window.
    fn page_down(&mut self, max_visible: usize);

    /// Moves the filter cursor left by one grapheme.
    fn move_cursor_left(&mut self);

    /// Moves the filter cursor right by one grapheme.
    fn move_cursor_right(&mut self);

    /// Clears the filter text and resets selection, cursor, and scroll offset.
    ///
    /// Used by the universal `CtrlClear` intent: pressing `<c-c>` in a picker
    /// with a non-empty filter clears it; subsequent presses close the picker.
    fn clear_filter(&mut self);

    /// Returns `true` when the filter text is empty.
    ///
    /// Companion to [`clear_filter`](Self::clear_filter) for the universal
    /// `CtrlClear` intent's branch decision.
    fn is_filter_empty(&self) -> bool;
}

impl<T: crate::PickerItem> PickerOps for crate::SelectionState<T> {
    fn insert_char(&mut self, ch: char) {
        crate::SelectionState::insert_char(self, ch);
    }

    fn insert_text(&mut self, text: &str) {
        crate::SelectionState::insert_text(self, text);
    }

    fn backspace(&mut self) {
        crate::SelectionState::backspace(self);
    }

    fn move_up(&mut self, max_visible: usize) {
        crate::SelectionState::move_up(self, max_visible);
    }

    fn move_down(&mut self, max_visible: usize) {
        crate::SelectionState::move_down(self, max_visible);
    }

    fn page_up(&mut self, max_visible: usize) {
        crate::SelectionState::page_up(self, max_visible);
    }

    fn page_down(&mut self, max_visible: usize) {
        crate::SelectionState::page_down(self, max_visible);
    }

    fn move_cursor_left(&mut self) {
        crate::SelectionState::move_cursor_left(self);
    }

    fn move_cursor_right(&mut self) {
        crate::SelectionState::move_cursor_right(self);
    }

    fn clear_filter(&mut self) {
        crate::SelectionState::clear_filter(self);
    }

    fn is_filter_empty(&self) -> bool {
        crate::SelectionState::filter(self).is_empty()
    }
}
