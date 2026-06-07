//! State for the arg input popup - collecting positional args for a lifecycle command.

use crate::common::line_input::LineInput;

/// State for the arg input popup - collecting positional args for a lifecycle command.
///
/// The editable text and cursor live in [`LineInput`] (shared with other popup
/// inputs) under the [`ArgInputState::text`] field; access via
/// `.text.input` / `.text.cursor_pos`.
#[derive(Debug, Clone, Default)]
pub struct ArgInputState {
    /// Which lifecycle we're collecting args for.
    pub lifecycle_name: String,
    /// The command template with `<param>` tokens for display.
    pub template_display: String,
    /// The editable text + cursor.
    pub text: LineInput,
}
