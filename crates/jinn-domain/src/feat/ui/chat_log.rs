//! Chat log - renders the full conversation history.
//!
//! A display-only component showing all messages exchanged in the active session.
//! Each entry type has a distinct visual style (user bold with `>`, system dark gray,
//! actor yellow, assistant cyan). Supports scrolling, selection highlighting,
//! and pinned entry indicators.
//!
//! The entry-to-lines pipeline itself lives in the `jinn-chat-log-view` slice:
//! it is pure, reading no application state. What remains here is the
//! `ChatLogElement` — the `UiElement` that reads `AppState`, resolves the
//! per-frame inputs, and drives scrolling, selection, and the gutter.

pub(crate) mod history;
#[cfg(test)]
mod history_tests;

pub use history::ChatLogElement;

use crate::common::AppUiRegistry;

/// Register chat log UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatLogElement::new()));
}
