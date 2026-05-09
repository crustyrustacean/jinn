//! User intents — what the user wants to do.
//!
//! Every keymap binding produces an [`Intent`] variant. Each intent has a
//! dedicated validator function that checks whether the intent can proceed
//! given the current [`AppState`].
//!
//! This crate has no TUI or async dependency. It supports headless and
//! script modes identically to the TUI mode.

pub mod intent;
pub mod validators;

pub use intent::Intent;
