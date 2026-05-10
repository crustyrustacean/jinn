//! Pinned context panel component — displays pinned context entries.
//!
//! Provides a side panel that lists all pinned entries with position badges,
//! supports j/k selection within the panel, and allows unpinning from the panel.
//!
//! Phase 5: Handler removed — pinned panel logic will be re-implemented in Phase 7.

pub mod element;
pub mod state;

pub use element::PinnedPanelElement;
pub use state::PinnedPanelState;
