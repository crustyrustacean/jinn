//! Provider family leftovers — the pieces that stayed kernel-side.
//!
//! The provider-selection family (actors, contracts, entry types, the
//! provider cell) lives in the `jinn-provider-selection` slice and its
//! `jinn-provider-selection-msg` crate. What remains here: the
//! streaming indicator (a session-*phase* visual), the chat-entry →
//! LLM-message converter (context-assembly's vocabulary), and the
//! provider UI registration.

pub mod entries_to_messages;
pub mod indicator;
pub mod llm_message;

#[cfg(test)]
mod entries_to_messages_tests;

pub use indicator::StreamingIndicatorElement;

use crate::common::AppUiRegistry;

/// Register provider UI elements.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StreamingIndicatorElement::new()));
}
