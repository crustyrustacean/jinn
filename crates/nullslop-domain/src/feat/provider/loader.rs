//! Provider picker loader — loads provider entries into picker state.

use crate::component::AppState;
use crate::common::services::Services;

use super::entries::{load_provider_entries, sorted_entries};

/// Loads provider entries into the picker state, ready for display.
///
/// Reads from the provider registry and model cache, applies available-first
/// sorting and active-provider promotion, then stores the entries via
/// `SelectionState::set_items`.
pub fn load_provider_picker_items(services: &Services, state: &mut AppState) {
    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(&registry, &api_keys, state.provider.model_cache.as_ref());
    let entries = sorted_entries(&all, "", &state.provider.active_provider);
    state.provider.provider_picker.set_items(entries);
}
