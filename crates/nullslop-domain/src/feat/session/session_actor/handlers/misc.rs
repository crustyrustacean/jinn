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
