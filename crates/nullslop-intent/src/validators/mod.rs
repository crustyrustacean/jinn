//! Validator functions for each [`Intent`](crate::Intent) variant.
//!
//! Every intent has a dedicated validator. Infallible intents return `()`.
//! Fallible intents return `Result<(), SpecificError>`.

pub mod app;
pub mod chat_entry;
pub mod chat_input;
pub mod dashboard;
pub mod navigation;
pub mod picker;
pub mod pinned_panel;
