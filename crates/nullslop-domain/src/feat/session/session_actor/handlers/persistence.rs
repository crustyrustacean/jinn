//! Persistence handlers — save and load session snapshots.

use std::collections::HashMap;

use jiff::Timestamp;

use super::super::super::PersistedSession;
use super::super::SessionPersistenceActor;
use crate::SessionLoadRequested;
use crate::feat::provider_infra::NO_PROVIDER_ID;
use crate::protocol::{ChatEntryKind, Command, PromptStrategyId};

impl SessionPersistenceActor {
    /// Saves the current state of a session to disk.
    ///
    /// Reads session data from shared state, derives title from first user
    /// message's first line, constructs a [`PersistedSession`], and writes
    /// via the store. Errors are logged as warnings — persistence failure
    /// must not break the user experience.
    pub(in crate::feat::session::session_actor) fn save_active_session(
        &self,
        session_id: &crate::protocol::SessionId,
    ) {
        let Some(store) = &self.store else {
            tracing::warn!("session-actor has no store — skipping save");
            return;
        };

        let persisted = {
            let state = self.state.read();
            let Some(session) = state.session.sessions.get(session_id) else {
                tracing::warn!(session_id = ?session_id, "session not found for save");
                return;
            };

            let title = session
                .history()
                .iter()
                .find_map(|entry| match &entry.kind {
                    ChatEntryKind::User(text) => Some(text.lines().next().unwrap_or("").to_owned()),
                    _ => None,
                })
                .unwrap_or_else(|| "Untitled Session".to_owned());

            PersistedSession {
                session_id: session_id.clone(),
                title,
                updated_at: Timestamp::now(),
                history: session.history().to_vec(),
                active_strategy: session.profile().strategy.clone(),
                model: session.profile().model.clone(),
                blobs: HashMap::new(),
            }
        };

        if let Err(e) = store.save(&persisted) {
            tracing::warn!(
                session_id = ?session_id,
                err = ?e,
                "failed to persist session"
            );
        }
    }

    /// Loads a full session from disk and sends back a `SessionLoadCompleted` command.
    pub(in crate::feat::session::session_actor) fn on_load_requested(
        &mut self,
        evt: &SessionLoadRequested,
        ctx: &crate::common::actor::ActorContext,
    ) {
        use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted as CompletedPayload;

        let Some(store) = &self.store else {
            tracing::warn!("session-actor has no store — dropping load request");
            return;
        };

        match store.load_full(evt.byte_offset) {
            Ok(Some(persisted)) => {
                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload {
                        session_id: persisted.session_id,
                        title: persisted.title,
                        history: persisted.history,
                        active_strategy: persisted.active_strategy,
                        model: persisted.model,
                        blobs: persisted.blobs,
                    },
                });
            }
            Ok(None) => {
                tracing::warn!(
                    byte_offset = evt.byte_offset,
                    "session load returned None at offset"
                );
                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload {
                        session_id: evt.session_id.clone(),
                        title: String::new(),
                        history: vec![],
                        active_strategy: PromptStrategyId::passthrough(),
                        model: crate::feat::provider_infra::NO_PROVIDER_ID.to_owned(),
                        blobs: std::collections::HashMap::new(),
                    },
                });
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load session");
                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload {
                        session_id: evt.session_id.clone(),
                        title: String::new(),
                        history: vec![],
                        active_strategy: PromptStrategyId::passthrough(),
                        model: crate::feat::provider_infra::NO_PROVIDER_ID.to_owned(),
                        blobs: std::collections::HashMap::new(),
                    },
                });
            }
        }
    }
}
