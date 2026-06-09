//! Provider picker loader - loads provider entries into picker state.

use crate::common::app_state::AppState;
use crate::common::services::Services;
use crate::feat::provider_infra;
use crate::feat::ui::picker_states::PickerExt;

use super::entries::{load_provider_entries, sorted_entries};

/// Sentinel provider ID for the "session default" compaction model entry.
///
/// Real provider IDs always contain a `/` (e.g., `anthropic/claude-sonnet-4-20250514`),
/// so a string without `/` and with a double-underscore prefix cannot collide.
pub(crate) const SESSION_DEFAULT_PROVIDER_ID: &str = "__session_default__";

/// Loads provider entries into the picker state, ready for display.
///
/// Reads from the provider registry and model cache, applies available-first
/// sorting and active-provider promotion, then stores the entries via
/// `SelectionState::set_items`.
pub fn load_provider_picker_items(services: &Services, state: &mut AppState) {
    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(
        &registry,
        &api_keys,
        state.provider.model_cache.as_ref(),
        &state.frontend.theme,
    );
    let active_model = state.active_session().profile().model.clone();
    let entries = sorted_entries(&all, "", &active_model);
    state.provider.provider_picker.set_items(entries);
}

/// Loads compaction model picker entries into the picker state, ready for display.
///
/// Prepends a "session default" sentinel entry (representing "no compaction model set;
/// fall back to the session's model") followed by all available provider entries.
/// Marks the active compaction model (or the sentinel if `compaction.model` is `None`).
///
/// # Panics
///
/// Panics if accessing the preferences subsystem fails.
pub fn load_compaction_model_picker_items(services: &Services, state: &mut AppState) {
    // Load preferences from service.
    let prefs = services.user_preferences_storage.read();

    // Build the sentinel entry.
    let active_compaction_model = prefs.compaction.model.clone();
    let sentinel_active = active_compaction_model.is_none();
    let sentinel = crate::protocol::PickerEntry {
        provider_id: SESSION_DEFAULT_PROVIDER_ID.to_owned(),
        name: String::new(),
        provider_name: String::new(),
        backend: String::new(),
        model: "session default".to_owned(),
        search_text: "session default".to_owned(),
        is_alias: false,
        alias_target: None,
        is_available: true,
        is_remote: false,
        is_active: sentinel_active,
        theme: state.frontend.theme.clone(),
    };

    // Load all provider entries using the existing infrastructure.
    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(
        &registry,
        &api_keys,
        state.provider.model_cache.as_ref(),
        &state.frontend.theme,
    );

    // Sort with the compaction model as the active provider.
    let active_id = active_compaction_model
        .as_deref()
        .unwrap_or(provider_infra::NO_PROVIDER_ID);
    let mut entries = sorted_entries(&all, "", active_id);

    // Prepend sentinel (always first).
    entries.insert(0, sentinel);

    state
        .frontend
        .compaction_model_picker_mut()
        .set_items(entries);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::test_services::TestServices;
    use crate::feat::provider_infra::{ProviderEntry, ProvidersConfig};

    #[rstest::rstest]
    fn load_compaction_model_picker_items_populates_picker() {
        // Kills: replace load_compaction_model_picker_items with ().
        // If the function were a no-op, the picker would remain empty.
        let services = TestServices::builder()
            .with_providers(ProvidersConfig {
                providers: vec![ProviderEntry {
                    name: "ollama".to_owned(),
                    backend: "ollama".to_owned(),
                    models: vec!["llama3".to_owned()],
                    base_url: Some("http://localhost:11434".to_owned()),
                    api_key_env: None,
                    requires_key: false,
                    extra_body: None,
                    context_length: None,
                }],
                aliases: vec![],
                default_provider: None,
            })
            .build();

        let mut state = AppState::default();

        // When loading compaction model picker items.
        load_compaction_model_picker_items(&services, &mut state);

        // Then the picker has entries (sentinel + 1 provider = 2).
        let items = state.frontend.compaction_model_picker().items();
        assert!(!items.is_empty(), "picker should not be empty");
        // First entry should be the sentinel "session default".
        assert_eq!(items[0].model, "session default");
        // Second entry should be the ollama/llama3 model.
        assert_eq!(items[1].provider_id, "ollama/llama3");
    }
}
