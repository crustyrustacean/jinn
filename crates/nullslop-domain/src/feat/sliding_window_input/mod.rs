//! Sliding window input popup — adjust the window size for the active session.
//!
//! Provides an inline numeric input (similar to TokenBudgetInput) that seeds with
//! the current session window size, accepts only digits, and on confirm updates
//! both the session profile and the global preferences.

pub mod intent;
pub mod render;
