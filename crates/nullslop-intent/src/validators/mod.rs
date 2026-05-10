//! Validator functions for each [`Intent`](crate::Intent) variant.
//!
//! Every intent has a dedicated validator. Infallible intents return `()`.
//! Fallible intents return `Result<(), SpecificError>`.

pub mod app;
pub mod chat_entry;
pub mod chat_input;
pub mod dashboard;
pub mod picker;
