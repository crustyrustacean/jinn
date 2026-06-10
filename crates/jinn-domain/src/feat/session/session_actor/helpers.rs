//! Shared helpers used across multiple handler concern modules.

use crate::common::actor::ActorContext;
use crate::feat::session::phase_machine::PhaseKind;
use crate::protocol::{Event, SessionId};

/// Emit a `SessionPhaseChanged` event if the phase actually changed.
///
/// Call this outside the write lock with the before/after phases captured inside.
pub(in crate::feat::session::session_actor) fn emit_phase_changed(
    ctx: &ActorContext,
    session_id: &SessionId,
    old_phase: impl Into<PhaseKind>,
    new_phase: impl Into<PhaseKind>,
) {
    let old_phase = old_phase.into();
    let new_phase = new_phase.into();
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

/// Emit a `HistoryAppended` event.
///
/// Call this outside the write lock.
pub(in crate::feat::session::session_actor) fn emit_history_appended(
    ctx: &ActorContext,
    session_id: &SessionId,
) {
    if let Err(e) = ctx.send_event(Event::HistoryAppended(
        crate::feat::session::protocol::history_appended::HistoryAppended {
            session_id: session_id.clone(),
        },
    )) {
        tracing::warn!(err = ?e, "failed to emit HistoryAppended");
    }
}

#[cfg(test)]
pub(super) async fn test_actor() -> super::SessionPersistenceActor {
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::context::strategy::token_estimator::TiktokenCounter;

    super::SessionPersistenceActor {
        state: State::new(AppState::default()),
        services: crate::common::services::Services::new_fake().await,
        counter: TiktokenCounter::o200k_base(),
        builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
        shell: "/bin/sh".to_owned(),
        lifecycle_child: None,
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
    summaries: Vec<crate::feat::session::session_summary::SessionSummary>,
    sessions: Vec<crate::feat::session::chat_session::ChatSessionState>,
    archived: std::sync::Mutex<Vec<crate::protocol::SessionId>>,
    saved: std::sync::Mutex<Vec<crate::feat::session::chat_session::ChatSessionState>>,
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
        #[expect(clippy::expect_used, reason = "test code")]
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
        #[expect(clippy::expect_used, reason = "test code")]
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
            #[expect(clippy::expect_used, reason = "test code")]
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
}

/// Builds a test actor with services and a populated store.
/// Returns (actor, store) so tests can inspect store state.
#[cfg(test)]
pub(super) async fn test_actor_with_store(
    sessions: Vec<crate::feat::session::chat_session::ChatSessionState>,
) -> (
    super::SessionPersistenceActor,
    std::sync::Arc<PopulatedFakeStore>,
) {
    let store = std::sync::Arc::new(PopulatedFakeStore::new(sessions));
    let services = crate::TestServices::builder()
        .session_store(crate::feat::session::SessionStoreService::new(
            store.clone(),
        ))
        .build();
    (
        super::SessionPersistenceActor {
            state: crate::common::state::State::new(crate::common::app_state::AppState::default()),
            services,
            counter: crate::feat::context::strategy::token_estimator::TiktokenCounter::o200k_base(),
            builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
            shell: "/bin/sh".to_owned(),
            lifecycle_child: None,
        },
        store,
    )
}
