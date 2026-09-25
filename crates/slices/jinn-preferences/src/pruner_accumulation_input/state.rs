//! State for the pruner accumulation threshold input popup.

use jinn_slices::LineInput;

/// The editable threshold and cursor for the pruner accumulation popup.
#[derive(Debug, Clone, Default)]
pub struct PrunerAccumulationInputState {
    /// The decimal threshold text and byte-offset cursor.
    pub text: LineInput,
}
