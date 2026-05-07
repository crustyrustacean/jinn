//! Session picker — browse and select saved sessions.
//!
//! Manages the picker overlay state for browsing persisted sessions.
//! The user can select a session to load, or press CTRL+N to start a new one.

pub mod entries;
mod handler;

use crate::{AppBus, AppUiRegistry};

/// Register the session picker component with the bus.
///
/// The picker has no UI element — it is rendered as an overlay in
/// `nullslop-tui/src/render.rs`.
pub(crate) fn register(bus: &mut AppBus, _registry: &mut AppUiRegistry) {
    handler::SessionPickerHandler.register(bus);
}
