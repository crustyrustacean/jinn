//! Token budget input popup — adjust the token budget for the active session.
//!
//! Provides an inline numeric input (similar to ArgInput) that seeds with
//! the current session budget, accepts only digits, and on confirm updates
//! both the session profile and the global preferences.

pub mod intent;
pub mod render;
