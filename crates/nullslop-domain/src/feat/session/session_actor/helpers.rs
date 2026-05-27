//! Shared helpers used across multiple handler concern modules.

use crate::common::actor::ActorContext;
use crate::feat::context::strategy::token_estimator::{CharRatioEstimator, estimate_entry_tokens};
use crate::feat::session::chat_session::{ChatSessionState, SessionPhase};
use crate::protocol::{Event, SessionId};

/// Emit a `SessionPhaseChanged` event if the phase actually changed.
///
/// Call this outside the write lock with the before/after phases captured inside.
pub(in crate::feat::session::session_actor) fn emit_phase_changed(
    ctx: &ActorContext,
    session_id: &SessionId,
    old_phase: SessionPhase,
    new_phase: SessionPhase,
) {
    if old_phase != new_phase
        && let Err(e) = ctx.send_event(Event::SessionPhaseChanged(
            crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase,
                new_phase,
            },
        ))
    {
        tracing::warn!(err = ?e, "failed to emit SessionPhaseChanged");
    }
}

/// Compute total estimated tokens for a session's history.
pub(in crate::feat::session::session_actor) fn estimate_total_tokens(
    session: &ChatSessionState,
) -> usize {
    let estimator = CharRatioEstimator;
    session
        .history()
        .iter()
        .map(|e| estimate_entry_tokens(&estimator, e))
        .sum()
}

/// Emit a `HistoryAppended` event with the total estimated tokens.
///
/// Call this outside the write lock with the pre-computed token count.
pub(in crate::feat::session::session_actor) fn emit_history_appended(
    ctx: &ActorContext,
    session_id: &SessionId,
    total_estimated_tokens: usize,
) {
    if let Err(e) = ctx.send_event(Event::HistoryAppended(
        crate::feat::session::protocol::history_appended::HistoryAppended {
            session_id: session_id.clone(),
            total_estimated_tokens,
        },
    )) {
        tracing::warn!(err = ?e, "failed to emit HistoryAppended");
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
        builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
        shell: "/bin/sh".to_owned(),
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
    pub(super) summaries: Vec<crate::feat::session::session_summary::SessionSummary>,
    pub(super) sessions: Vec<crate::feat::session::chat_session::ChatSessionState>,
    pub(super) archived: std::sync::Mutex<Vec<crate::protocol::SessionId>>,
    pub(super) saved: std::sync::Mutex<Vec<crate::feat::session::chat_session::ChatSessionState>>,
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
                parent_session: s.parent_session().clone(),
            })
            .collect();
        Self {
            summaries,
            sessions,
            archived: std::sync::Mutex::new(Vec::new()),
            saved: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(super) fn last_saved_session(
        &self,
        id: &crate::protocol::SessionId,
    ) -> Option<crate::feat::session::chat_session::ChatSessionState> {
        self.saved
            .lock()
            .expect("lock")
            .iter()
            .rev()
            .find(|s| s.session_id() == id)
            .cloned()
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
        session: &crate::feat::session::chat_session::ChatSessionState,
    ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
    {
        self.saved.lock().expect("lock").push(session.clone());
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
            self.archived.lock().expect("lock").push(session_id.clone());
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

    async fn load_judge_sessions_for_origin(
        &self,
        origin_session_id: &crate::protocol::SessionId,
    ) -> Result<
        Vec<crate::feat::session::chat_session::ChatSessionState>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(self
            .sessions
            .iter()
            .filter(|s| {
                s.judge()
                    .as_ref()
                    .is_some_and(|m| m.origin_session == *origin_session_id)
            })
            .cloned()
            .collect())
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
            builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
            shell: "/bin/sh".to_owned(),
        },
        store,
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::protocol::ChatEntry;

    #[test]
    fn estimate_total_tokens_returns_nonzero_for_session_with_entries() {
        // Given a session with multiple entries.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello world from user"));
        session.push_entry(ChatEntry::assistant("hello from assistant"));

        // When estimating total tokens.
        let tokens = estimate_total_tokens(&session);

        // Then the result is a meaningful sum (not 0 or 1).
        assert!(
            tokens > 1,
            "estimate_total_tokens should return a sum > 1 for a session with entries, got {tokens}"
        );
    }
}
