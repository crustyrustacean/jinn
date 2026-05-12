//! Chat log — renders the full conversation history.
//!
//! A display-only component showing all messages exchanged in the active session.
//! Each entry type has a distinct visual style (user bold with `>`, system dark gray,
//! actor yellow, assistant cyan). Supports scrolling, selection highlighting,
//! and pinned entry indicators.

pub mod element;

pub use element::ChatLogElement;

use crate::common::AppUiRegistry;

/// Register chat log UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatLogElement));
}
