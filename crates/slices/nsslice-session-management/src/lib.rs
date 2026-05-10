//! Session management slice — session creation, model refresh, and prompt template rescan.
//!
//! Handles creating new sessions, refreshing the model list from the
//! active provider, and rescanning prompt templates. No element —
//! rendering stays in `nullslop-tui`.

pub mod intent;
pub mod validator;
