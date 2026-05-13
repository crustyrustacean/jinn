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

    /// Removes the character before the cursor.
    fn backspace(&mut self);

    /// Moves the selection up by one item.
    fn move_up(&mut self, max_visible: usize);

    /// Moves the selection down by one item.
    fn move_down(&mut self, max_visible: usize);

    /// Moves the filter cursor left by one grapheme.
    fn move_cursor_left(&mut self);

    /// Moves the filter cursor right by one grapheme.
    fn move_cursor_right(&mut self);
}

impl<T: crate::PickerItem> PickerOps for crate::SelectionState<T> {
    fn insert_char(&mut self, ch: char) {
        crate::SelectionState::insert_char(self, ch);
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

    fn move_cursor_left(&mut self) {
        crate::SelectionState::move_cursor_left(self);
    }

    fn move_cursor_right(&mut self) {
        crate::SelectionState::move_cursor_right(self);
    }
}
