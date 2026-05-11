//! Chat input box slice — message composition UI, validation, and intent handling.
//!
//! Co-locates everything about the chat input box:
//!
//! - **Element** — renders the input prompt with cursor positioning and mode-aware styling.
//! - **Validator** — validates message submission, autocomplete confirmation, and normal escape.
//! - **Intent** — handles 16 intents: 13 chat-input intents (character insertion, deletion,
//!   submission, autocomplete, cursor movement), plus EnterInsertMode, EnterNormalMode,
//!   and NormalEscape.
//!
//! State (`ChatInputBoxState`) stays in `nullslop-component` to avoid circular dependencies.

pub mod autocomplete_render;
pub mod element;
pub mod intent;
pub mod validator;

pub use element::ChatInputBoxElement;

use nullslop_component::AppUiRegistry;

/// Register chat input box UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatInputBoxElement));
}
