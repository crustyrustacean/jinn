//! Pinned panel slice — context entry pinning UI, validation, and intent handling.
//!
//! Co-locates everything about the pinned context panel:
//!
//! - **Element** — renders pinned entries with position badges and selection highlighting.
//! - **Validator** — validates pin/unpin actions (checks selection, checks entries exist).
//! - **Intent** — handles all 11 pinned-panel intents (toggle, open, close, select,
//!   unpin, pin top/bottom/relative, pin cycle).
//!
//! State (`PinnedPanelState`) stays in `nullslop-component` to avoid circular dependencies.

pub mod element;
pub mod intent;
pub mod validator;

pub use element::PinnedPanelElement;

use nullslop_component::AppUiRegistry;

/// Register pinned panel UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(PinnedPanelElement));
}
