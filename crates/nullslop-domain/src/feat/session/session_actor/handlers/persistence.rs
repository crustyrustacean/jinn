//! Persistence handlers — load session snapshots.

use crate::protocol::{Command, PromptStrategyId};
use crate::SessionLoadRequested;

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
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
