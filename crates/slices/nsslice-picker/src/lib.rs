//! Picker slice — picker navigation, filtering, confirmation, and scope toggling.
//!
//! Handles all picker intents (open, insert char, backspace, confirm, move,
//! cursor movement, toggle scope filter) and their validators.
//! No element — rendering stays in `nullslop-tui`.

pub mod intent;
pub mod validator;
