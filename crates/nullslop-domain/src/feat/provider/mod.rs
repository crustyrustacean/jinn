//! Provider slice — LLM provider selection, model discovery, and streaming UI.
//!
//! Provides [`ProviderState`] for tracking the active provider, model cache,
//! and provider picker state. Also contains the provider actor, discover actor,
//! and UI elements (streaming indicator, queue display).

pub mod discover_actor;
pub mod entries;
pub mod entries_to_messages;
pub mod indicator;
pub mod llm_message;
pub mod loader;
pub mod picker_entry;
pub mod protocol;
pub mod provider_actor;
pub mod queue_element;
pub mod render;

#[cfg(test)]
mod render_tests;

#[cfg(test)]
mod entries_tests;

pub use indicator::StreamingIndicatorElement;
pub use queue_element::QueueDisplayElement;

use crate::common::AppUiRegistry;

use crate::PickerEntry;

/// Provider selection state — owned by the provider-actor.
///
/// Written to exclusively by `ProviderActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct ProviderState {
    /// Last known model cache from discovery.
    /// OWNER: provider-actor (updates from ModelsRefreshed event).
    pub model_cache: Option<crate::feat::provider_infra::ModelCache>,

    /// When the model list was last refreshed (UTC).
    /// OWNER: provider-actor (updates from ModelsRefreshed event).
    pub last_refreshed_at: Option<jiff::Timestamp>,

    /// Provider picker state (items, filter text, selection index).
    /// OWNER: provider-actor (loads entries via LoadProviderPickerEntries),
    ///        IntentHandler (navigates picker, reads selected item).
    pub provider_picker: nullslop_selection_widget::SelectionState<PickerEntry>,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            model_cache: None,
            last_refreshed_at: None,
            provider_picker: nullslop_selection_widget::SelectionState::new(),
        }
    }
}

/// Register provider UI elements.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StreamingIndicatorElement::new()));
    registry.register(Box::new(QueueDisplayElement));
}
