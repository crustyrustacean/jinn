//! Chat input box slice — message composition UI, validation, and intent handling.
//!
//! Co-locates everything about the chat input box:
//!
//! - **Element** — renders the input prompt with cursor positioning and mode-aware styling.
//! - **Validator** — validates message submission, autocomplete confirmation, and interrupt.
//! - **Intent** — handles 13 chat-input intents (character insertion, deletion, submission,
//!   autocomplete, cursor movement).
//!
//! State (`ChatInputBoxState`) stays in `nullslop-component` to avoid circular dependencies.
//!
//! **Note**: `handle_interrupt` and `handle_set_mode` stay in `nullslop-intent` because
//! they're cross-cutting (cancel streams, transition modes). They call into this slice's
//! validators but orchestrate domain logic themselves.

pub mod element;
pub mod intent;
pub mod validator;

pub use element::ChatInputBoxElement;

use nullslop_component::AppUiRegistry;

/// Register chat input box UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatInputBoxElement));
}
