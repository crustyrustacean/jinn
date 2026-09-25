//! Provider family leftovers — the pieces that stayed kernel-side.
//!
//! The provider-selection family (actors, contracts, entry types, the
//! provider cell) lives in the `jinn-provider-selection` slice and its
//! `jinn-provider-selection-msg` crate. What remains here: the
//! streaming indicator (a session-*phase* visual) and the provider UI
//! registration. The chat-entry → LLM-message converter lives in
//! `jinn-llm-support`.

pub mod indicator;

pub use indicator::StreamingIndicatorElement;

use crate::common::AppUiRegistry;

/// Register provider UI elements.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StreamingIndicatorElement::new()));
}
