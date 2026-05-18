//! Chat log — renders the full conversation history.
//!
//! A display-only component showing all messages exchanged in the active session.
//! Each entry type has a distinct visual style (user bold with `>`, system dark gray,
//! actor yellow, assistant cyan). Supports scrolling, selection highlighting,
//! and pinned entry indicators.

pub(crate) mod actor;
pub(crate) mod assistant;
pub(crate) mod compaction;
pub(crate) mod error_entry;
pub(crate) mod info;
pub(crate) mod line_count_cache;
pub(crate) mod markdown;
pub(crate) mod renderer;
#[cfg(test)]
mod renderer_tests;
pub(crate) mod shared;
pub(crate) mod skill;
pub(crate) mod system;
pub(crate) mod table;
pub(crate) mod thinking;
pub(crate) mod tool_call;
pub(crate) mod tool_result;
pub(crate) mod user;

pub use renderer::ChatLogElement;
pub use shared::GUTTER_WIDTH;

use crate::common::AppUiRegistry;

/// Register chat log UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatLogElement::new()));
}
