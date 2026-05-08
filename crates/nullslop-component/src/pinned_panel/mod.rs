//! Pinned context panel component — displays pinned context entries.
//!
//! Provides a side panel that lists all pinned entries with position badges,
//! supports j/k selection within the panel, and allows unpinning from the panel.

pub mod element;
pub(crate) mod handler;
pub mod state;

pub use element::PinnedPanelElement;
pub use state::PinnedPanelState;

use crate::{AppBus, AppUiRegistry};

/// Registers the pinned panel handler and UI element.
pub(crate) fn register(bus: &mut AppBus, registry: &mut AppUiRegistry) {
    handler::PinnedPanelHandler.register(bus);
    registry.register(Box::new(PinnedPanelElement));
}
