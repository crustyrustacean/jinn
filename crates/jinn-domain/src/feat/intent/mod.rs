//! User intents - what the user wants to do.
//!
//! Every keymap binding produces an [`Intent`] variant. Each intent has a
//! dedicated validator function that checks whether the intent can proceed
//! given the current [`AppState`].
//!
//! The [`IntentHandler`] processes all 55 intents: it validates each intent,
//! then acts on it - mutating [`AppState`], setting TUI signals, and returning
//! commands/events for the actor system.
//!
//! This crate has no TUI or async dependency. It supports headless and
//! script modes identically to the TUI mode.

pub mod handler;
pub mod intent;

pub use crate::IntentResult;
pub use handler::IntentHandler;
pub use intent::Intent;
