//! The slice-owned UI element registrations.
//!
//! The registrations themselves live in `jinn_tui::ui_elements`, which
//! the test builder shares: a second copy here would drift from the
//! one the app runs with.

pub use jinn_tui::build_ui_registry;
