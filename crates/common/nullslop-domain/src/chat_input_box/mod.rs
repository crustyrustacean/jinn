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
pub mod element;
pub mod intent;
pub mod state;
pub mod validator;

// Re-export state types for convenience.
pub use state::AutocompleteState;
pub use state::ChatInputBoxState;

// Re-export protocol's AutocompleteMatch (used by intent handler and autocomplete render).
pub use nsslice_chat_input_box_protocol::AutocompleteMatch;

// Re-export element for registration.
pub use element::ChatInputBoxElement;

use nullslop_component::AppUiRegistry;

/// Register chat input box UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatInputBoxElement));
}

#[cfg(test)]
mod tests;
