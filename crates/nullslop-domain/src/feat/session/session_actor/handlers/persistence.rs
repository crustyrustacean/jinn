//! Persistence handlers — save and load session snapshots.

use super::super::SessionPersistenceActor;
use crate::SessionLoadRequested;
use crate::feat::context::protocol::command::{RestoreStrategyState, SwitchPromptStrategy};
use crate::protocol::Command;

impl SessionPersistenceActor {
    /// Saves the current state of a session to disk.
    ///
    /// Reads session data from shared state, touches the timestamp, and writes
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

        {
            let mut state = self.state.write();
            let Some(session) = state.session.sessions.get_mut(session_id) else {
                tracing::warn!(session_id = ?session_id, "session not found for save");
                return;
            };
            session.touch();

            if let Err(e) = store.save(session) {
                tracing::warn!(
                    session_id = ?session_id,
                    err = ?e,
                    "failed to persist session"
                );
            }
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
            Ok(Some(session)) => {
                let strategy_id = session.active_strategy().clone();
                let strategy_blob = session
                    .strategy_state()
                    .get(&strategy_id)
                    .and_then(|s| serde_json::to_value(s).ok())
                    .unwrap_or(serde_json::json!({}));

                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload { session },
                });
                // Also emit RestoreStrategyState and SwitchPromptStrategy.
                let _ = ctx.send_command(Command::RestoreStrategyState {
                    payload: RestoreStrategyState {
                        session_id: evt.session_id.clone(),
                        strategy_id: strategy_id.clone(),
                        blob: strategy_blob,
                    },
                });
                let _ = ctx.send_command(Command::SwitchPromptStrategy {
                    payload: SwitchPromptStrategy {
                        session_id: evt.session_id.clone(),
                        strategy_id,
                    },
                });
            }
            Ok(None) => {
                tracing::warn!(
                    byte_offset = evt.byte_offset,
                    "session load returned None at offset"
                );
                // Create an empty session with the requested ID.
                let mut session = crate::feat::session::chat_session::ChatSessionState::new();
                session.set_session_id(evt.session_id.clone());
                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload { session },
                });
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load session");
                let mut session = crate::feat::session::chat_session::ChatSessionState::new();
                session.set_session_id(evt.session_id.clone());
                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload { session },
                });
            }
        }
    }
}
