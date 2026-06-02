//! State for the rename workflow input popup - editing a workflow label.

/// State for the rename workflow input popup - editing a workflow label.
#[derive(Debug, Clone, Default)]
pub struct RenameWorkflowInputState {
    /// User's raw input text.
    pub input: String,
    /// Byte offset for cursor position in the input.
    pub cursor_pos: usize,
}
