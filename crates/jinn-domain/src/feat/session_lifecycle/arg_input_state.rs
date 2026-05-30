//! State for the arg input popup - collecting positional args for a lifecycle command.

/// State for the arg input popup - collecting positional args for a lifecycle command.
#[derive(Debug, Clone, Default)]
pub struct ArgInputState {
    /// Which lifecycle we're collecting args for.
    pub lifecycle_name: String,
    /// The command template with `<param>` tokens for display.
    pub template_display: String,
    /// User's raw input text.
    pub input: String,
    /// Byte offset for cursor position in the input.
    pub cursor_pos: usize,
}
