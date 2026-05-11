//! Persistence handlers — save and load session snapshots.

use jiff::Timestamp;

use super::super::super::PersistedSession;
use crate::protocol::{Command, PromptStrategyId, SessionLoadRequested, SessionSaveRequested};

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// Constructs a [`PersistedSession`] from the event payload and saves it.
    ///
    /// Errors are logged as warnings — persistence failure must not break
    /// the user experience.
    pub(in crate::feat::session::actor) fn on_save_requested(
        &mut self,
        evt: &SessionSaveRequested,
    ) {
        let Some(store) = &self.store else {
            tracing::warn!("session-actor has no store — dropping save request");
            return;
        };

        let persisted = PersistedSession {
            session_id: evt.session_id.clone(),
            title: evt.title.clone(),
            updated_at: Timestamp::now(),
            history: evt.history.clone(),
            active_strategy: evt.active_strategy.clone(),
            blobs: evt.blobs.clone(),
        };

        if let Err(e) = store.save(&persisted) {
            tracing::warn!(
                session_id = ?evt.session_id,
                err = ?e,
                "failed to persist session"
            );
        }
    }

    /// Loads a full session from disk and sends back a `SessionLoadCompleted` command.
    pub(in crate::feat::session::actor) fn on_load_requested(
        &mut self,
        evt: &SessionLoadRequested,
        ctx: &crate::common::actor::ActorContext,
    ) {
        use crate::protocol::session::SessionLoadCompleted as CompletedPayload;

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
                        blobs: std::collections::HashMap::new(),
                    },
                });
            }
        }
    }
}
