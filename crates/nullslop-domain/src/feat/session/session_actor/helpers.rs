//! Shared helpers used across multiple handler concern modules.

use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::protocol::command::AssemblePrompt;
use crate::protocol::{ChatEntry, Command, Event, SessionId};

use super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// Drain queued messages into a new turn: push each entry, then emit
    /// `AssemblePrompt` with the full session history.
    pub(in crate::feat::session::session_actor) async fn start_turn_from_queued(
        &self,
        session_id: &SessionId,
        entries: &[ChatEntry],
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            for entry in entries {
                session.push_entry(entry.clone());
            }
            session.begin_sending();
        }

        let (history, model_name) = {
            let state = self.state.read();
            let session = state.session(session_id);
            (session.history().to_vec(), session.profile().model.clone())
        };

        if let Err(e) = ctx.send_command(Command::AssemblePrompt(AssemblePrompt {
            session_id: session_id.clone(),
            history,
            tools: vec![],
            model_name,
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit AssemblePrompt from queue drain");
        }

        // Emit ChatEntrySubmitted for each queued entry.
        for entry in entries {
            if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
                session_id: session_id.clone(),
                entry: entry.clone(),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted for queued message");
            }
        }

        // Persist the queued entries for crash safety.
        self.save_active_session(session_id).await;
    }
}

#[cfg(test)]
pub(super) fn test_actor() -> super::SessionPersistenceActor {
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::context::strategy::token_estimator::TiktokenCounter;

    super::SessionPersistenceActor {
        state: State::new(AppState::default()),
        services: None,
        store: None,
        counter: TiktokenCounter::o200k_base(),
    }
}

#[cfg(test)]
pub(super) fn test_context() -> (
    std::sync::Arc<crate::common::actor::RecordingSink>,
    crate::common::actor::ActorContext,
) {
    use crate::common::actor::{ActorContext, RecordingSink};

    let sink = std::sync::Arc::new(RecordingSink::new());
    let ctx = ActorContext::new("test-session-actor", sink.clone());
    (sink, ctx)
}
