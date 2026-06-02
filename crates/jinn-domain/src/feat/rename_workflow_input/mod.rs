//! Rename workflow input popup - rename the selected workflow label from the sidebar.
//!
//! Provides a text input popup that seeds with the current workflow label,
//! accepts any characters, and on confirm updates the workflow label and
//! triggers persistence.

pub mod intent;
pub mod render;
pub mod state;
