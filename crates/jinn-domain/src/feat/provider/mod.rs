//! Provider slice - LLM provider selection, model discovery, and streaming UI.
//!
//! Provides [`ProviderState`] for tracking the active provider, model cache,
//! and provider picker state. Also contains the provider actor, discover actor,
//! and UI elements (streaming indicator).

pub mod discover_actor;
pub mod entries;
pub mod entries_to_messages;
pub mod indicator;
pub mod llm_message;
pub mod loader;
pub mod picker_entry;
pub mod protocol;
pub mod provider_actor;

pub mod render;

#[cfg(test)]
mod render_tests;

#[cfg(test)]
mod discover_actor_tests;

#[cfg(test)]
mod entries_tests;

#[cfg(test)]
mod entries_to_messages_tests;

pub use indicator::StreamingIndicatorElement;


use crate::common::AppUiRegistry;

use crate::PickerEntry;

/// Provider selection state - owned by the provider-actor.
///
/// Written to exclusively by `ProviderActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct ProviderState {
    /// Last known model cache from discovery.
    /// OWNER: provider-actor (updates from ModelsRefreshed / ModelCacheLoaded events).
    pub model_cache: Option<crate::feat::provider_infra::ModelCache>,

    /// Provider picker state (items, filter text, selection index).
    /// OWNER: provider-actor (loads entries via LoadProviderPickerEntries),
    ///        IntentHandler (navigates picker, reads selected item).
    pub provider_picker: jinn_selection_widget::SelectionState<PickerEntry>,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            model_cache: None,
            provider_picker: jinn_selection_widget::SelectionState::new(),
        }
    }
}

/// Register provider UI elements.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StreamingIndicatorElement::new()));
}
