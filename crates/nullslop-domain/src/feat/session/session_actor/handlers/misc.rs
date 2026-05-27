//! Miscellaneous handlers — model refresh display and session picker loading.
//!
//! Handles pushing model refresh results as transient markdown entries to the chat log,
//! and loading session picker entries from the session store into app state.

use crate::feat::provider::protocol::event::ModelsRefreshed;

use super::super::SessionPersistenceActor;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::protocol::ChatEntry;

impl SessionPersistenceActor {
    /// Pushes a transient markdown entry after model refresh.
    ///
    /// Builds a markdown table from the refresh results and pushes it directly
    /// to session state. Does NOT emit `PushChatEntry` — transient entries
    /// are not persisted.
    #[allow(clippy::unused_self)]
    pub(in crate::feat::session::session_actor) fn on_models_refreshed(
        &self,
        event: &ModelsRefreshed,
    ) {
        let content = if event.results.is_empty() && event.errors.is_empty() {
            "Models refreshed: no providers found".to_owned()
        } else {
            build_models_refresh_table(event)
        };

        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            session.push_entry(ChatEntry::transient(content));
        }
    }

    /// Loads session picker entries from the session store into `AppState`.
    pub(in crate::feat::session::session_actor) async fn handle_load_session_picker_entries(
        &self,
        _payload: &LoadSessionPickerEntries,
    ) {
        if let Some(ref store) = self.store {
            let theme = {
                let state = self.state.read();
                state.frontend.theme.clone()
            };
            let entries =
                crate::feat::session::entries::load_session_entries_from_store(store, &theme).await;
            let mut state = self.state.write();
            state.frontend.session_picker.set_items(entries);
        }
    }
}

/// Builds a markdown table string from the models refresh event.
///
/// Format:
/// ```markdown
/// | Provider | Models | Status |
/// |----------|--------|--------|
/// | ollama   | 5      | ✅     |
/// | openai   | 0      | ❌ API key not resolved |
/// ```
fn build_models_refresh_table(event: &ModelsRefreshed) -> String {
    // Collect all provider names and sort alphabetically.
    let mut all_providers: Vec<&str> = event
        .results
        .keys()
        .chain(event.errors.keys())
        .map(std::string::String::as_str)
        .collect();
    all_providers.sort_unstable();
    all_providers.dedup();

    let mut rows = Vec::new();
    for provider in all_providers {
        if let Some(models) = event.results.get(provider) {
            rows.push(format!("| {provider} | {} | ✅ |", models.len()));
        } else if let Some(err) = event.errors.get(provider) {
            rows.push(format!("| {provider} | 0 | ❌ {err} |"));
        }
    }

    let mut table = String::new();
    table.push_str("| Provider | Models | Status |\n");
    table.push_str("|----------|--------|--------|\n");
    for row in rows {
        table.push_str(&row);
        table.push('\n');
    }

    table
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::super::super::helpers::{test_actor, test_actor_with_store};
    use crate::feat::provider::protocol::event::ModelsRefreshed;
    use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
    use crate::protocol::{ChatEntryKind, SessionId};
    use nullslop_provider::ModelInfo;
    use std::collections::HashMap;

    // --- on_models_refreshed ---

    #[test]
    fn on_models_refreshed_pushes_transient_entry() {
        // Given a session actor.
        let actor = test_actor();
        let session_id = SessionId::new();

        // When refreshing models with some results.
        let mut results = HashMap::new();
        results.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: Some(8192),
            }],
        );
        actor.on_models_refreshed(&ModelsRefreshed {
            session_id: session_id.clone(),
            results,
            errors: HashMap::new(),
        });

        // Then a transient entry with a table was pushed.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.history().len(), 1);
        let entry = &session.history()[0];
        assert!(matches!(&entry.kind, ChatEntryKind::Transient(t) if t.contains("ollama")));
    }

    #[test]
    fn on_models_refreshed_empty_results_shows_no_providers_message() {
        // Given a session actor.
        let actor = test_actor();
        let session_id = SessionId::new();

        // When refreshing models with empty results AND empty errors.
        actor.on_models_refreshed(&ModelsRefreshed {
            session_id: session_id.clone(),
            results: HashMap::new(),
            errors: HashMap::new(),
        });

        // Then a transient entry with "no providers found" message was pushed.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.history().len(), 1);
        let entry = &session.history()[0];
        assert!(
            matches!(&entry.kind, ChatEntryKind::Transient(t) if t.contains("no providers found")),
            "expected 'no providers found' message, got {:?}",
            entry.kind
        );
    }

    #[test]
    fn on_models_refreshed_with_errors_shows_table() {
        // Given a session actor.
        let actor = test_actor();
        let session_id = SessionId::new();

        // When refreshing models with errors but no results.
        let mut errors = HashMap::new();
        errors.insert("openai".to_owned(), "API key not resolved".to_owned());
        actor.on_models_refreshed(&ModelsRefreshed {
            session_id: session_id.clone(),
            results: HashMap::new(),
            errors,
        });

        // Then a transient entry with a table (not the "no providers" message) was pushed.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let entry = &session.history()[0];
        assert!(
            matches!(&entry.kind, ChatEntryKind::Transient(t) if t.contains("openai") && t.contains("API key not resolved")),
            "expected table with error, got {:?}",
            entry.kind
        );
    }

    // --- build_models_refresh_table ---

    #[test]
    fn build_models_refresh_table_includes_provider_and_model_count() {
        // Given a refresh event with results.
        let mut results = HashMap::new();
        results.insert(
            "ollama".to_owned(),
            vec![
                ModelInfo { id: "llama3".to_owned(), context_length: Some(8192) },
                ModelInfo { id: "phi3".to_owned(), context_length: None },
            ],
        );
        let event = ModelsRefreshed {
            session_id: SessionId::new(),
            results,
            errors: HashMap::new(),
        };

        // When building the table.
        let table = super::build_models_refresh_table(&event);

        // Then the table contains the provider name and correct model count.
        assert!(table.contains("ollama"), "expected provider name in table");
        assert!(table.contains("2"), "expected model count in table");
        assert!(table.contains("✅"), "expected success indicator");
    }

    // --- handle_load_session_picker_entries ---

    #[tokio::test]
    async fn handle_load_session_picker_entries_loads_from_store() {
        // Given an actor with a store containing a session.
        let session = crate::feat::session::chat_session::ChatSessionState::new();
        let (actor, _store) = test_actor_with_store(vec![session]);

        // When loading session picker entries.
        actor
            .handle_load_session_picker_entries(&LoadSessionPickerEntries)
            .await;

        // Then the session picker has entries (at least one from the stored session).
        let state = actor.state.read();
        assert!(
            !state.frontend.session_picker.items().is_empty(),
            "expected session picker to have entries after loading from store"
        );
    }
}
