//! State for the rename session input popup — editing a session title.

/// State for the rename session input popup — editing a session title.
#[derive(Debug, Clone, Default)]
pub struct RenameSessionInputState {
    /// User's raw input text.
    pub input: String,
    /// Byte offset for cursor position in the input.
    pub cursor_pos: usize,
}
