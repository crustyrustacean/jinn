//! Chat input box — where the user composes and sends messages.
//!
//! This component manages the text input experience end to end: handling keystrokes,
//! displaying the in-progress message, tracking the input buffer, and switching
//! between browsing and typing modes.
//!
//! Phase 5: Handler removed — input logic is now handled directly by
//! the IntentHandler. Only the UI element is registered.

pub mod element;
pub mod state;

pub use element::ChatInputBoxElement;
pub use state::{AutocompleteMatch, AutocompleteState, ChatInputBoxState};
