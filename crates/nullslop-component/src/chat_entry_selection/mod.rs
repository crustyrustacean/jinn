//! Chat entry selection component — handles j/k navigation in the chat log.
//!
//! This component wires \[`ChatEntrySelectionHandler`] into the bus so that
//! `ChatEntrySelectNext`, `ChatEntrySelectPrev`, and `ChatEntrySelectCancel`
//! commands are dispatched to the active session's selection methods.

pub mod handler;

pub(crate) use handler::ChatEntrySelectionHandler;

use crate::{AppBus, AppUiRegistry};

/// Register the chat entry selection handler.
pub(crate) fn register(bus: &mut AppBus, _registry: &mut AppUiRegistry) {
    ChatEntrySelectionHandler.register(bus);
}
