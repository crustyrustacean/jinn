//! Session picker — browse and select saved sessions.
//!
//! Manages the picker overlay state for browsing persisted sessions.
//! The user can select a session to load, or press CTRL+N to start a new one.
//!
//! Phase 5: Handler removed — session loading will be re-implemented in Phase 7.

pub mod entries;

pub use entries::SessionEntry;
