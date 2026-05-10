//! Chat input box — where the user composes and sends messages.
//!
//! This component manages the text input experience end to end: handling keystrokes,
//! displaying the in-progress message, tracking the input buffer, and switching
//! between browsing and typing modes.
//!
//! The rendering element and intent handling are in the `nsslice-chat-input-box` slice crate.
//! Only state types remain here.

pub mod state;

pub use state::{AutocompleteMatch, AutocompleteState, ChatInputBoxState};
