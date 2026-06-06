//! State for the rename session input popup - editing a session title.

use crate::common::line_input::LineInput;

/// State for the rename session input popup - editing a session title.
///
/// The editable text and cursor live in [`LineInput`] (shared with other popup
/// inputs) under the [`RenameSessionInputState::text`] field; access via
/// `.text.input` / `.text.cursor_pos`.
#[derive(Debug, Clone, Default)]
pub struct RenameSessionInputState {
    /// The editable text + cursor.
    pub text: LineInput,
}
