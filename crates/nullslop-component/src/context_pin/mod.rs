//! Context pin component — handles pin/unpin commands for chat entries.
//!
//! This component wires \[`ContextPinHandler`] into the bus so that
//! `PinChatEntry` and `UnpinChatEntry` commands are dispatched to
//! `ChatSessionState::pin_entry` and `ChatSessionState::unpin_entry`.

pub mod handler;

pub(crate) use handler::ContextPinHandler;

use crate::{AppBus, AppUiRegistry};

/// Register the context pin handler.
pub(crate) fn register(bus: &mut AppBus, _registry: &mut AppUiRegistry) {
    ContextPinHandler.register(bus);
}
