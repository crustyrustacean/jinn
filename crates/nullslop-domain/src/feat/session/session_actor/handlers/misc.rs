//! Miscellaneous handlers — model refresh display and session picker loading.
//!
//! Handles pushing model refresh results as table entries to the chat log,
//! and loading session picker entries from the session store into app state.

use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::provider::protocol::event::ModelsRefreshed;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::protocol::{ChatEntry, Command, TableData};

use super::super::SessionPersistenceActor;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;

impl SessionPersistenceActor {
    /// Pushes a table entry after model refresh.
    ///
    /// Emits `PushChatEntry` commands so the entries are persisted.
    #[allow(clippy::unused_self)]
    pub(in crate::feat::session::session_actor) fn on_models_refreshed(
        &self,
        event: &ModelsRefreshed,
        ctx: &ActorContext,
    ) {
        // No providers at all — push a simple system entry.
        if event.results.is_empty() && event.errors.is_empty() {
            if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: event.session_id.clone(),
                entry: ChatEntry::system("Models refreshed: no providers found"),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for models refresh");
            }
            return;
        }

        let headers = vec![
            Span::raw("Provider"),
            Span::raw("Model Count"),
            Span::raw("Status"),
        ];

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
                rows.push(vec![
                    Span::raw(provider.to_owned()),
                    Span::raw(models.len().to_string()),
                    Span::styled("\u{2705}".to_owned(), Style::default().fg(Color::Green)),
                ]);
            } else if let Some(err) = event.errors.get(provider) {
                rows.push(vec![
                    Span::raw(provider.to_owned()),
                    Span::raw("0".to_owned()),
                    Span::styled(format!("\u{274c} {err}"), Style::default().fg(Color::Red)),
                ]);
            }
        }

        let data = TableData { headers, rows };
        if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: event.session_id.clone(),
            entry: ChatEntry::table(data),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for models refresh");
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
