//! Global slice — quit, toggle which-key, and interrupt.
//!
//! Handles cross-cutting application actions: quitting, toggling the
//! which-key popup, and interrupting the active stream or clearing input.
//! No element — rendering stays in `jinn-tui`.

pub mod intent;
pub mod validator;
