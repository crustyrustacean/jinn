use crate::BusService;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::history_appended::HistoryAppended;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::protocol::SessionId;

/// Emit a `SessionPhaseChanged` event if the phase actually changed.
///
/// Call this outside the write lock with the before/after phases captured inside.
pub(in crate::feat::session::session_actor) async fn emit_phase_changed(
    bus: &BusService,
    session_id: &SessionId,
    old_phase: impl Into<PhaseKind>,
    new_phase: impl Into<PhaseKind>,
) {
    let old_phase = old_phase.into();
    let new_phase = new_phase.into();
    if old_phase != new_phase {
        bus.publish(SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase,
            new_phase,
        })
        .await;
    }
}

/// Emit a `HistoryAppended` event.
///
/// Call this outside the write lock.
pub(in crate::feat::session::session_actor) async fn emit_history_appended(
    bus: &BusService,
    session_id: &SessionId,
) {
    bus.publish(HistoryAppended {
        session_id: session_id.clone(),
    })
    .await;
}

#[cfg(test)]
pub(crate) async fn test_actor() -> super::SessionPersistenceActor {
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::context::strategy::token_estimator::TiktokenCounter;
    use jinn_token_count_msg::HistoryWorkerChatEntryTokenCache;

    super::SessionPersistenceActor {
        state: State::new(AppState::default_with_scope_focus()),
        cap: crate::common::tcaps::mint::mint_session_cap(),
        frontend_cap: crate::common::tcaps::mint::mint_frontend_cap(),
        services: crate::common::services::Services::new_fake().await,
        counter: TiktokenCounter::o200k_base(),
        token_cache: HistoryWorkerChatEntryTokenCache::default(),
        image_converter: test_image_converter(),
    }
}

#[cfg(test)]
#[cfg(test)]
pub(crate) async fn test_actor_recording() -> (
    super::SessionPersistenceActor,
    crate::common::services::BusAudit,
) {
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::context::strategy::token_estimator::TiktokenCounter;
    use jinn_token_count_msg::HistoryWorkerChatEntryTokenCache;

    let (bus, audit) = crate::common::services::BusService::new_recording();
    let services = crate::common::services::Services::new_fake_with_bus(bus).await;
    // Dispatch paths assemble through the trouper context-assembly
    // service; spawn the test-crate stub so the live-value ask crosses
    // no compilation boundary (see assembly_test_bridge docs).
    let _ = crate::feat::context::assembly_test_bridge::ensure_spawned(&services.trouper_system);

    (
        super::SessionPersistenceActor {
            state: State::new(AppState::default()),
            cap: crate::common::tcaps::mint::mint_session_cap(),
            frontend_cap: crate::common::tcaps::mint::mint_frontend_cap(),
            services,
            counter: TiktokenCounter::o200k_base(),
            token_cache: HistoryWorkerChatEntryTokenCache::default(),
            image_converter: test_image_converter(),
        },
        audit,
    )
}

#[cfg(test)]
use parking_lot::Mutex;

#[cfg(test)]
/// A fake session store that returns pre-loaded sessions for testing.
pub(crate) struct PopulatedFakeStore {
    summaries: parking_lot::Mutex<Vec<crate::feat::session::session_summary::SessionSummary>>,
    sessions: parking_lot::Mutex<Vec<crate::feat::session::chat_session::ChatSessionState>>,
    archived: parking_lot::Mutex<Vec<crate::protocol::SessionId>>,
    saved: parking_lot::Mutex<Vec<crate::feat::session::chat_session::ChatSessionState>>,
    fail_load_summaries: parking_lot::Mutex<bool>,
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
                project: s.project().map(std::path::Path::to_path_buf),
            })
            .collect();
        Self {
            summaries: Mutex::new(summaries),
            sessions: Mutex::new(sessions),
            archived: Mutex::new(Vec::new()),
            saved: Mutex::new(Vec::new()),
            fail_load_summaries: Mutex::new(false),
        }
    }

    pub(super) fn last_saved_session(
        &self,
        id: &crate::protocol::SessionId,
    ) -> Option<crate::feat::session::chat_session::ChatSessionState> {
        self.saved
            .lock()
            .iter()
            .rev()
            .find(|s| s.session_id() == id)
            .cloned()
    }

    /// The store's current readable snapshot for `id` (what `load_session`
    /// / `fork` would see) — not the save journal, the persisted rows.
    pub(super) fn stored_session(
        &self,
        id: &crate::protocol::SessionId,
    ) -> Option<crate::feat::session::chat_session::ChatSessionState> {
        self.sessions
            .lock()
            .iter()
            .find(|s| s.session_id() == id)
            .cloned()
    }

    /// Snapshot of all IDs passed to `set_archived`/`set_archived_many`
    /// (append order, duplicates preserved).
    pub(super) fn archived_ids(&self) -> Vec<crate::protocol::SessionId> {
        self.archived.lock().clone()
    }

    /// Replaces the summaries the store reports (simulates store-only rows).
    pub(super) fn set_summaries(
        &self,
        summaries: Vec<crate::feat::session::session_summary::SessionSummary>,
    ) {
        *self.summaries.lock() = summaries;
    }

    /// Makes `load_summaries` fail (simulates a store read error).
    pub(super) fn set_fail_load_summaries(&self, fail: bool) {
        *self.fail_load_summaries.lock() = fail;
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
        self.saved.lock().push(session.clone());
        // Upsert into the readable sessions vec (the real store persists the
        // session so later reads — fork, load — see it).
        let mut sessions = self.sessions.lock();
        match sessions
            .iter_mut()
            .find(|s| s.session_id() == session.session_id())
        {
            Some(existing) => *existing = session.clone(),
            None => sessions.push(session.clone()),
        }
        Ok(())
    }

    async fn load_summaries(
        &self,
    ) -> Result<
        Vec<crate::feat::session::session_summary::SessionSummary>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        if *self.fail_load_summaries.lock() {
            return Err(error_stack::Report::new(
                crate::feat::session::session_store::SessionStoreError,
            ));
        }
        Ok(self.summaries.lock().clone())
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
            .lock()
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
        source_session_id: &crate::protocol::SessionId,
        at_ordinal: usize,
    ) -> Result<
        crate::protocol::SessionId,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        // Mirror the SQL fork's contract: error when the source is not in the
        // store, otherwise copy entries up to and including `at_ordinal` into
        // a new child session (fresh id, parent set).
        let sessions = self.sessions.lock();
        let Some(source) = sessions
            .iter()
            .find(|s| s.session_id() == source_session_id)
            .cloned()
        else {
            return Err(error_stack::Report::new(
                crate::feat::session::session_store::SessionStoreError,
            ));
        };
        drop(sessions);
        let new_id = crate::protocol::SessionId::new();
        let mut forked = crate::feat::session::chat_session::ChatSessionState::new();
        forked.set_session_id(new_id.clone());
        forked.set_parent_session(source_session_id.clone());
        if let Some(title) = source.title() {
            forked.set_title(title.to_owned());
        }
        for entry in source.history().iter().take(at_ordinal + 1) {
            forked.push_entry(entry.clone());
        }
        self.sessions.lock().push(forked);
        // Keep summaries in sync so follow-up loads see the fork.
        self.summaries
            .lock()
            .push(crate::feat::session::session_summary::SessionSummary {
                session_id: new_id.clone(),
                title: source.title().unwrap_or("Untitled Session").to_owned(),
                updated_at: *source.updated_at(),
                created_at: *source.created_at(),
                session_state: crate::feat::session::chat_session::SessionState::Loaded,
                parent_session: Some(source_session_id.clone()),
                project: source.project().map(std::path::Path::to_path_buf),
            });
        Ok(new_id)
    }

    async fn set_archived(
        &self,
        session_id: &crate::protocol::SessionId,
        archived: bool,
    ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
    {
        if archived {
            self.archived.lock().push(session_id.clone());
        }
        Ok(())
    }

    async fn set_archived_many(
        &self,
        session_ids: &[crate::protocol::SessionId],
        archived: bool,
    ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
    {
        if archived {
            let mut archived = self.archived.lock();
            archived.extend(session_ids.iter().cloned());
        }
        Ok(())
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<
        Vec<crate::feat::session::session_summary::SessionSummary>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(self.summaries.lock().clone())
    }

    async fn dirty_session_ids(
        &self,
    ) -> Result<
        Vec<crate::protocol::SessionId>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(Vec::new())
    }

    async fn reindex_session_chunk(
        &self,
        _session_id: &crate::protocol::SessionId,
        _max_entries: usize,
    ) -> Result<bool, error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
    {
        Ok(true)
    }

    async fn pending_dirty_count(
        &self,
    ) -> Result<usize, error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
    {
        Ok(0)
    }

    async fn search(
        &self,
        _params: crate::feat::session_search::SearchParams,
    ) -> Result<
        crate::feat::session_search::SearchOutcome,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(crate::feat::session_search::SearchOutcome {
            total_matches: 0,
            per_session: Vec::new(),
            hits: Vec::new(),
        })
    }

    async fn fetch_window(
        &self,
        _session_id: &crate::protocol::SessionId,
        _anchor: &crate::protocol::ChatEntryId,
        _context: usize,
    ) -> Result<
        Option<crate::feat::session_search::TranscriptWindow>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(None)
    }

    async fn fetch_tail(
        &self,
        _session_id: &crate::protocol::SessionId,
        _limit: usize,
    ) -> Result<
        Option<crate::feat::session_search::TranscriptWindow>,
        error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
    > {
        Ok(None)
    }
}

#[cfg(test)]
pub(crate) async fn test_actor_with_store_recording(
    sessions: Vec<crate::feat::session::chat_session::ChatSessionState>,
) -> (
    super::SessionPersistenceActor,
    std::sync::Arc<PopulatedFakeStore>,
    crate::common::services::BusAudit,
) {
    let store = std::sync::Arc::new(PopulatedFakeStore::new(sessions));
    let (bus, audit) = crate::common::services::BusService::new_recording();
    let services = crate::TestServices::builder()
        .session_store(crate::feat::session::SessionStoreService::new(
            store.clone(),
        ))
        .with_bus(bus)
        .build();
    (
        super::SessionPersistenceActor {
            state: crate::common::state::State::new(crate::common::app_state::AppState::default()),
            cap: crate::common::tcaps::mint::mint_session_cap(),
            frontend_cap: crate::common::tcaps::mint::mint_frontend_cap(),
            services,
            counter: crate::feat::context::strategy::token_estimator::TiktokenCounter::o200k_base(),
            token_cache: jinn_token_count_msg::HistoryWorkerChatEntryTokenCache::default(),
            image_converter: test_image_converter(),
        },
        store,
        audit,
    )
}

/// Constructs an [`ImageConverterService`] for tests. Uses a no-op
/// converter so tests don't spawn ImageMagick. Actors that test the
/// conversion path inject their own converter.
#[cfg(test)]
pub(in crate::feat::session::session_actor) fn test_image_converter()
-> crate::feat::image_convert::ImageConverterService {
    crate::feat::image_convert::ImageConverterService::unavailable()
}
