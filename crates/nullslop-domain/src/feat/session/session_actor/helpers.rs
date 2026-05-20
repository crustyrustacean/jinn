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

// --- Shared test store helpers ---

/// A fake session store that returns pre-loaded sessions for testing.
#[cfg(test)]
pub(super) struct PopulatedFakeStore {
    pub(super) summaries:
        Vec<crate::feat::session::session_summary::SessionSummary>,
    pub(super) sessions: Vec<crate::feat::session::chat_session::ChatSessionState>,
    pub(super) archived: std::sync::Mutex<Vec<crate::protocol::SessionId>>,
}

#[cfg(test)]
impl PopulatedFakeStore {
    pub(super) fn new(sessions: Vec<crate::feat::session::chat_session::ChatSessionState>) -> Self {
        let summaries = sessions
            .iter()
            .map(|s| crate::feat::session::session_summary::SessionSummary {
                session_id: s.session_id().clone(),
                title: s.title().unwrap_or("Untitled Session").to_owned(),
                updated_at: *s.updated_at(),
                created_at: *s.created_at(),
                session_state: crate::feat::session::chat_session::SessionState::Loaded,
            })
            .collect();
        Self {
            summaries,
            sessions,
            archived: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(super) fn was_archived(&self, id: &crate::protocol::SessionId) -> bool {
        self.archived.lock().expect("lock").contains(id)
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl crate::feat::session::session_store::SessionStore for PopulatedFakeStore {
    fn name(&self) -> &'static str {
        "populated-fake"
    }

    async fn save(
        &self,
        _session: &crate::feat::session::chat_session::ChatSessionState,
    ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
    {
        Ok(())
    }

    async fn load_summaries(
        &self,
    ) -> Result<
        Vec<crate::feat::session::session_summary::SessionSummary>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(self.summaries.clone())
    }

    async fn load_session(
        &self,
        session_id: &crate::protocol::SessionId,
    ) -> Result<
        Option<crate::feat::session::chat_session::ChatSessionState>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(self
            .sessions
            .iter()
            .find(|s| s.session_id() == session_id)
            .cloned())
    }

    async fn delete(
        &self,
        _session_id: &crate::protocol::SessionId,
    ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
    {
        Ok(())
    }

    async fn fork(
        &self,
        _source_session_id: &crate::protocol::SessionId,
        _at_ordinal: usize,
    ) -> Result<
        crate::protocol::SessionId,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(crate::protocol::SessionId::new())
    }

    async fn set_archived(
        &self,
        session_id: &crate::protocol::SessionId,
        archived: bool,
    ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
    {
        if archived {
            self.archived
                .lock()
                .expect("lock")
                .push(session_id.clone());
        }
        Ok(())
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<
        Vec<crate::feat::session::session_summary::SessionSummary>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(self.summaries.clone())
    }
}

/// Builds a test actor with services and a populated store.
/// Returns (actor, store) so tests can inspect store state.
#[cfg(test)]
pub(super) fn test_actor_with_store(
    sessions: Vec<crate::feat::session::chat_session::ChatSessionState>,
) -> (
    super::SessionPersistenceActor,
    std::sync::Arc<PopulatedFakeStore>,
) {
    let store = std::sync::Arc::new(PopulatedFakeStore::new(sessions));
    let services = crate::TestServices::builder()
        .session_store(super::SessionStoreService::new(store.clone()))
        .build();
    let service_store = services.session_store.clone();
    (
        super::SessionPersistenceActor {
            state: crate::common::state::State::new(crate::common::app_state::AppState::default()),
            services: Some(services),
            store: Some(service_store),
            counter: crate::feat::context::strategy::token_estimator::TiktokenCounter::o200k_base(),
        },
        store,
    )
}
