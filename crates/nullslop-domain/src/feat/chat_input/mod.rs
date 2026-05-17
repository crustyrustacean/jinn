//! Chat input box — message composition UI, validation, and intent handling.
//!
//! Co-locates everything about the chat input box:
//!
//! - **State** — `ChatInputBoxState` holds the input buffer, cursor, and autocomplete state.
//! - **Element** — renders the input prompt with cursor positioning and mode-aware styling.
//! - **Validator** — validates message submission, autocomplete confirmation, and normal escape.
//! - **Intent** — handles 16 intents: 13 chat-input intents (character insertion, deletion,
//!   submission, autocomplete, cursor movement), plus EnterInsertMode, EnterNormalMode,
//!   and NormalEscape.
//! - **Autocomplete render** — renders the prompt template autocomplete popup overlay.

pub mod autocomplete_render;

#[cfg(test)]
mod autocomplete_render_tests;
pub mod element;
pub mod intent;
pub mod protocol;
pub mod slash_command;
pub mod state;
pub mod validator;

// Re-export state types for convenience.
pub use state::AutocompleteState;
pub use state::ChatInputBoxState;
pub use state::autocomplete::AutocompleteTrigger;

/// A single match for the prompt template autocomplete popup.
#[derive(Debug, Clone)]
pub struct AutocompleteMatch {
    /// The template name (e.g. `"code-review"`).
    pub name: String,
    /// Short human-readable description for the popup.
    pub description: String,
}

// Re-export element for registration.
pub use element::ChatInputBoxElement;

use crate::common::AppUiRegistry;

/// Register chat input box UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatInputBoxElement));
}

#[cfg(test)]
mod tests;
