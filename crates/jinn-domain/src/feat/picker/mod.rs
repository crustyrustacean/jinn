//! Picker - fuzzy search picker for providers, strategies, and sessions.
//!
//! Handles all picker intents (open, insert char, backspace, confirm, move,
//! cursor movement), their validators, and rendering.

pub mod intent;

pub mod picker_kind;
pub mod render;

pub mod style;
pub mod validator;

pub use picker_kind::PickerKind;
