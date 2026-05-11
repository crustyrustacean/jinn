//! Provider protocol — shared state types for the provider slice.
//!
//! Provides [`ProviderState`] for tracking the active provider, model cache,
//! and provider picker state.

use nullslop_protocol::PickerEntry;
use nullslop_providers::NO_PROVIDER_ID;

/// Provider selection state — owned by the provider-actor.
///
/// Written to exclusively by `ProviderActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct ProviderState {
    /// The currently active provider. Always set — starts as `NO_PROVIDER_ID`.
    /// OWNER: provider-actor (sets on ProviderSwitch),
    ///        src/app.rs (sets initial value at startup).
    pub active_provider: String,

    /// Last known model cache from discovery.
    /// OWNER: provider-actor (updates from ModelsRefreshed event).
    pub model_cache: Option<nullslop_providers::ModelCache>,

    /// When the model list was last refreshed (UTC).
    /// OWNER: provider-actor (updates from ModelsRefreshed event).
    pub last_refreshed_at: Option<jiff::Timestamp>,

    /// Provider picker state (items, filter text, selection index).
    /// OWNER: provider-actor (loads entries via LoadPickerEntries),
    ///        IntentHandler (navigates picker, reads selected item).
    pub provider_picker: nullslop_selection_widget::SelectionState<PickerEntry>,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            active_provider: NO_PROVIDER_ID.to_owned(),
            model_cache: None,
            last_refreshed_at: None,
            provider_picker: nullslop_selection_widget::SelectionState::new(),
        }
    }
}
