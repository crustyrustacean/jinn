//! Workflow source node input editing.
//!
//! Provides intent handlers and validators for editing workflow source node
//! output data. Reuses [`ChatInputBoxState`](crate::feat::chat_input::ChatInputBoxState)
//! for buffer management while keeping intent handling and rendering fully
//! decoupled from the chat input system.

pub mod intent;
pub mod validator;

#[cfg(test)]
mod validator_tests;

#[cfg(test)]
mod intent_tests;
