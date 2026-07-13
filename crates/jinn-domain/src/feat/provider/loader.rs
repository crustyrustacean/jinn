//! Provider picker loader - loads provider entries into picker state.

use super::entries::{load_provider_entries, promote_selected_to_top, sorted_entries};
use crate::common::services::Services;
use crate::common::tcaps::provider::{
    FrontendProviderPickerWrite, ModelCacheWrite, ProviderPickerWrite, ProviderView,
};
use crate::feat::provider_infra;
use crate::feat::reasoning::{ReasoningEffort, ReasoningEffortEntry, resolve_effort};
use crate::feat::session::model_selection::ModelSelection;

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
pub fn load_provider_picker_items(services: &Services, view: &mut ProviderView<'_>) {
    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(
        &registry,
        &api_keys,
        view.provider.model_cache(),
        view.provider_frontend.theme(),
    );

    let model_selection = view.session.active_session().profile().model.clone();
    let active_model = model_selection.display_str().to_owned();
    let mut entries = sorted_entries(&all, "", &active_model);

    // Pre-check entries matching the current model selection, but only when
    // the picker is in alloy mode. Single mode never builds checkmarks.
    if view.provider.is_alloy_mode() {
        pre_check_active_models(&mut entries, &model_selection);
        promote_selected_to_top(&mut entries);
    }

    view.provider.set_provider_picker_items(entries);
}

/// Sets `selected = true` on entries matching the current model selection.
///
/// For `Single`, checks the one matching entry. For `Alloy`, checks all member entries.
pub(crate) fn pre_check_active_models(
    entries: &mut [crate::protocol::PickerEntry],
    selection: &ModelSelection,
) {
    let model_ids: Vec<&str> = match selection {
        ModelSelection::Single(s) => vec![s],
        ModelSelection::Alloy { models, .. } => models.iter().map(String::as_str).collect(),
    };
    for entry in entries.iter_mut() {
        if model_ids.iter().any(|id| *id == entry.provider_id) {
            entry.selected = true;
        }
    }
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
pub fn load_compaction_model_picker_items(services: &Services, view: &mut ProviderView<'_>) {
    // Load preferences from service.
    let prefs = services.user_preferences_storage.read();

    // Build the sentinel entry.
    let active_compaction_model = prefs.compaction.model;
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
        selected: false,
        theme: view.provider_frontend.theme().clone(),
    };

    // Load all provider entries using the existing infrastructure.
    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(
        &registry,
        &api_keys,
        view.provider.model_cache(),
        view.provider_frontend.theme(),
    );

    // Sort with the compaction model as the active provider.
    let active_id = active_compaction_model
        .as_deref()
        .unwrap_or(provider_infra::NO_PROVIDER_ID);
    let mut entries = sorted_entries(&all, "", active_id);

    // Prepend sentinel (always first).
    entries.insert(0, sentinel);

    view.provider_frontend
        .set_compaction_model_picker_items(entries);
}

/// Human-readable description for each effort variant.
///
/// Display-only; the wire value is [`ReasoningEffort::as_str`].
fn effort_description(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::Max => "Maximum effort",
        ReasoningEffort::Xhigh => "Extra-high effort",
        ReasoningEffort::High => "High effort",
        ReasoningEffort::Medium => "Medium effort",
        ReasoningEffort::Low => "Low effort",
        ReasoningEffort::Minimal => "Minimal effort",
        ReasoningEffort::None => "Skip reasoning",
    }
}

/// All seven effort variants in declaration order.
///
/// Kept in sync with the `ReasoningEffort` enum. Serves as the single source of
/// truth for the picker's row order.
const ALL_EFFORTS: [ReasoningEffort; 7] = [
    ReasoningEffort::Max,
    ReasoningEffort::Xhigh,
    ReasoningEffort::High,
    ReasoningEffort::Medium,
    ReasoningEffort::Low,
    ReasoningEffort::Minimal,
    ReasoningEffort::None,
];

/// Loads reasoning effort entries into the picker state, ready for display.
///
/// Builds one entry per `ReasoningEffort` variant (7 total), marking as active
/// the variant resolved by [`resolve_effort`] from the session's own effort
/// (seeded from the global at session creation). When the session has no effort
/// set, no entry is active.
pub fn load_reasoning_effort_picker_items(view: &mut ProviderView<'_>) {
    // The session owns its effort (seeded from the global at creation).
    let active = resolve_effort(view.session.active_session().profile().reasoning_effort);

    let entries = ALL_EFFORTS
        .iter()
        .map(|&effort| {
            let name = effort.as_str().to_owned();
            ReasoningEffortEntry {
                effort,
                name,
                description: effort_description(effort).to_owned(),
                is_active: active == Some(effort),
                theme: view.provider_frontend.theme().clone(),
            }
        })
        .collect::<Vec<_>>();

    view.provider_frontend
        .set_reasoning_effort_picker_items(entries);
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
    use crate::common::tcaps::provider::ProviderView;
    use crate::feat::provider_infra::{ProviderEntry, ProvidersConfig};
    use crate::feat::session::model_selection::{AlloyStrategy, ModelSelection};
    use crate::feat::ui::picker_states::PickerExt;

    #[rstest::rstest]
    fn load_compaction_model_picker_items_populates_picker() {
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
        load_compaction_model_picker_items(
            &services,
            &mut ProviderView::from_app_state_for_test(&mut state),
        );

        // Then the picker has entries (sentinel + 1 provider = 2).
        let items = state.frontend.compaction_model_picker().items();
        assert!(!items.is_empty(), "picker should not be empty");
        // First entry should be the sentinel "session default".
        assert_eq!(items[0].model, "session default");
        // Second entry should be the ollama/llama3 model.
        assert_eq!(items[1].provider_id, "ollama/llama3");
    }

    #[rstest::rstest]
    fn load_reasoning_effort_picker_items_populates_seven_entries() {
        // Given a default AppState (session owns its effort).
        let mut state = AppState::default();

        // When loading reasoning effort picker items.
        load_reasoning_effort_picker_items(&mut ProviderView::from_app_state_for_test(&mut state));

        // Then the picker has exactly 7 entries (one per variant).
        let items = state.frontend.reasoning_effort_picker().items();
        assert_eq!(items.len(), 7, "one entry per ReasoningEffort variant");
    }

    #[rstest::rstest]
    fn reasoning_effort_loader_marks_session_effort_active() {
        // Given a session with a reasoning_effort of Low.
        let mut state = AppState::default();
        state.active_session_mut().profile_mut().reasoning_effort =
            Some(crate::ReasoningEffort::Low);

        // When loading.
        load_reasoning_effort_picker_items(&mut ProviderView::from_app_state_for_test(&mut state));

        // Then only the Low entry is marked active.
        let items = state.frontend.reasoning_effort_picker().items();
        let low = items
            .iter()
            .find(|e| e.effort == crate::ReasoningEffort::Low)
            .expect("Low entry");
        assert!(low.is_active, "Low should be active (session's own effort)");
        assert_eq!(
            items.iter().filter(|e| e.is_active).count(),
            1,
            "exactly one active entry"
        );
    }

    #[rstest::rstest]
    fn reasoning_effort_loader_ignores_global_default() {
        // Given no session effort but a global default of High.
        let services = TestServices::builder().build();
        {
            let mut app_state = services.app_state_storage.read();
            app_state.reasoning_effort = Some(crate::ReasoningEffort::High);
            services
                .app_state_storage
                .save(&app_state)
                .expect("save app state");
        }
        let mut state = AppState::default();

        // When loading.
        load_reasoning_effort_picker_items(&mut ProviderView::from_app_state_for_test(&mut state));

        // Then no entry is active — the picker reads only the session's own
        // effort, which is None here. The global seeds new sessions; it never
        // affects the active row.
        let items = state.frontend.reasoning_effort_picker().items();
        assert_eq!(
            items.iter().filter(|e| e.is_active).count(),
            0,
            "global default must not mark any entry active"
        );
    }

    #[rstest::rstest]
    fn reasoning_effort_loader_marks_no_entry_active_when_session_unset() {
        // Given a session with no effort set.
        let mut state = AppState::default();

        // When loading.
        load_reasoning_effort_picker_items(&mut ProviderView::from_app_state_for_test(&mut state));

        // Then no entry is active (resolve_effort returns None).
        let items = state.frontend.reasoning_effort_picker().items();
        assert_eq!(
            items.iter().filter(|e| e.is_active).count(),
            0,
            "no active entry when session has no effort"
        );
    }

    #[rstest::rstest]
    fn load_picker_with_single_model_checks_matching_entry() {
        // Given a state with a single model and a provider picker.
        let services = TestServices::builder()
            .with_providers(ProvidersConfig {
                providers: vec![ProviderEntry {
                    name: "ollama".to_owned(),
                    backend: "ollama".to_owned(),
                    models: vec!["llama3".to_owned(), "mistral".to_owned()],
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
        state
            .active_session_mut()
            .set_model(ModelSelection::Single("ollama/llama3".to_owned()));

        state.provider.set_alloy_mode(true);
        // When loading provider picker items.
        load_provider_picker_items(
            &services,
            &mut ProviderView::from_app_state_for_test(&mut state),
        );

        // Then the entry matching the session model has selected = true.
        let items = state.provider.provider_picker.items();
        let llama = items
            .iter()
            .find(|e| e.provider_id == "ollama/llama3")
            .expect("llama3");
        assert!(llama.selected, "llama3 should be selected");

        // And the other entry is not selected.
        let mistral = items
            .iter()
            .find(|e| e.provider_id == "ollama/mistral")
            .expect("mistral");
        assert!(!mistral.selected, "mistral should not be selected");
    }

    #[rstest::rstest]
    fn load_picker_with_alloy_checks_all_member_entries() {
        // Given a state with an alloy of 2 models.
        let services = TestServices::builder()
            .with_providers(ProvidersConfig {
                providers: vec![ProviderEntry {
                    name: "ollama".to_owned(),
                    backend: "ollama".to_owned(),
                    models: vec![
                        "llama3".to_owned(),
                        "mistral".to_owned(),
                        "gemma".to_owned(),
                    ],
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
        state.active_session_mut().set_model(ModelSelection::Alloy {
            models: vec!["ollama/llama3".to_owned(), "ollama/mistral".to_owned()],
            strategy: AlloyStrategy::RoundRobin { index: 0 },
        });

        state.provider.set_alloy_mode(true);
        // When loading provider picker items.
        load_provider_picker_items(
            &services,
            &mut ProviderView::from_app_state_for_test(&mut state),
        );

        // Then both alloy members are selected.
        let items = state.provider.provider_picker.items();
        let llama = items
            .iter()
            .find(|e| e.provider_id == "ollama/llama3")
            .expect("llama3");
        assert!(llama.selected, "llama3 should be selected");

        let mistral = items
            .iter()
            .find(|e| e.provider_id == "ollama/mistral")
            .expect("mistral");
        assert!(mistral.selected, "mistral should be selected");

        // And the non-member is not selected.
        let gemma = items
            .iter()
            .find(|e| e.provider_id == "ollama/gemma")
            .expect("gemma");
        assert!(!gemma.selected, "gemma should not be selected");
    }

    #[rstest::rstest]
    fn pre_checked_alloy_members_sort_to_top() {
        // Given a state with an alloy of llama3 and mistral, plus gemma as non-member.
        let services = TestServices::builder()
            .with_providers(ProvidersConfig {
                providers: vec![ProviderEntry {
                    name: "ollama".to_owned(),
                    backend: "ollama".to_owned(),
                    models: vec![
                        "gemma".to_owned(),
                        "llama3".to_owned(),
                        "mistral".to_owned(),
                    ],
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
        state.active_session_mut().set_model(ModelSelection::Alloy {
            models: vec!["ollama/llama3".to_owned(), "ollama/mistral".to_owned()],
            strategy: AlloyStrategy::RoundRobin { index: 0 },
        });

        state.provider.set_alloy_mode(true);
        // When loading picker items.
        load_provider_picker_items(
            &services,
            &mut ProviderView::from_app_state_for_test(&mut state),
        );

        // Then selected entries (llama3, mistral) appear before non-selected (gemma).
        let items = state.provider.provider_picker.items();
        let llama_idx = items
            .iter()
            .position(|e| e.provider_id == "ollama/llama3")
            .expect("llama3");
        let mistral_idx = items
            .iter()
            .position(|e| e.provider_id == "ollama/mistral")
            .expect("mistral");
        let gemma_idx = items
            .iter()
            .position(|e| e.provider_id == "ollama/gemma")
            .expect("gemma");
        assert!(llama_idx < gemma_idx, "llama3 should sort above gemma");
        assert!(mistral_idx < gemma_idx, "mistral should sort above gemma");
    }
}
