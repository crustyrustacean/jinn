//! Input buffer for the chat input box.
//!
//! Holds the user's in-progress message — the text they have typed but not yet sent.
//! Tracks cursor position as a grapheme-cluster index so that insert and delete
//! operations work correctly at any position in the buffer.

pub mod state;

// Re-export state types for convenience.
pub use state::AutocompleteState;
pub use state::ChatInputBoxState;

/// A single match shown in the autocomplete popup.
///
/// Lightweight snapshot — stores only the name and description for rendering.
/// The full template body is looked up from the store only when needed
/// (e.g. double-`$` expansion).
#[derive(Debug, Clone)]
pub struct AutocompleteMatch {
    /// The template name (e.g. `"code-review"`).
    pub name: String,
    /// Short human-readable description for the popup.
    pub description: String,
}

#[cfg(test)]
mod tests;
