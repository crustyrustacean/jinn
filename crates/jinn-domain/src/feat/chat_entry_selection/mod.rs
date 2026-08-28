//! Chat entry selection slice - navigate and pin chat log entries.
//!
//! Handles selecting the next/previous chat entry and pinning the
//! selected entry. No element - rendering stays in `jinn-tui`.

pub mod ignore_sweep;
pub mod intent;
pub mod isolate;
pub mod validator;
