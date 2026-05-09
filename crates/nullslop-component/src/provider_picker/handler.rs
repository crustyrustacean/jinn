//! Provider picker handler (stubbed).
//!
//! Phase 5: This handler will be deleted entirely.
//! Stubbed — Picker* commands were removed from the Command enum.

#![allow(missing_docs)]

use nullslop_component_core::define_handler;
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct PickerHandler;

    commands {}

    events {}
}

/// Loads provider entries into the picker state, ready for display.
///
/// Reads from the provider registry and model cache, applies available-first
/// sorting and active-provider promotion, then stores the entries via
/// `SelectionState::set_items`.
pub fn load_provider_picker_items(services: &Services, state: &mut AppState) {
    use crate::provider_picker::entries::{load_provider_entries, sorted_entries};

    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(&registry, &api_keys, state.model_cache.as_ref());
    let entries = sorted_entries(&all, "", &state.active_provider);
    state.provider_picker.set_items(entries);
}
