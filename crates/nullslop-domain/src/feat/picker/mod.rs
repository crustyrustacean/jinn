//! Picker — fuzzy search picker for providers, strategies, keymaps, and sessions.
//!
//! Handles all picker intents (open, insert char, backspace, confirm, move,
//! cursor movement, toggle scope filter), their validators, and rendering.

pub mod intent;
pub mod keymap_entries;
pub mod keymap_entry;
pub mod render;
pub mod strategy_entries;
pub mod strategy_entry;
pub mod validator;
