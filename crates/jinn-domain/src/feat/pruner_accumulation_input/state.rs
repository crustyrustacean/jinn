//! State for the pruner accumulation threshold input popup.

use crate::common::line_input::LineInput;

/// State for the pruner accumulation threshold input popup — editing a number.
///
/// The editable text and cursor live in [`LineInput`] (shared with other popup
/// inputs) under the [`PrunerAccumulationInputState::text`] field; access via
/// `.text.input` / `.text.cursor_pos`.
#[derive(Debug, Clone, Default)]
pub struct PrunerAccumulationInputState {
    /// The editable text + cursor.
    pub text: LineInput,
}
