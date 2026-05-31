//! Rename session input popup - rename the selected session from the sidebar.
//!
//! Provides a text input popup that seeds with the current session title,
//! accepts any characters, and on confirm updates the session title and
//! triggers persistence.

pub mod intent;
pub mod render;
pub mod state;
