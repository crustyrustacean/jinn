//! Shared test doubles for the session-store slice.
//!
//! The hydration pool's tests need a store whose reads can be gated, held
//! open, or failed on demand — the same seam the store actor's startup tests
//! use — so the fake lives here rather than being duplicated per test module.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test support")]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use error_stack::Report;
use jinn_core_types::SessionId;
use jinn_domain::feat::session::{SessionStore, SessionStoreError};
use jinn_domain::protocol::ChatEntryId;
use jinn_session_state::{ChatSessionState, SessionSnapshot};
use jinn_session_store_msg::{
    SearchOutcome, SearchParams, SessionState, SessionSummary, TranscriptWindow,
};

/// Polls `condition` until it holds, or the budget runs out.
pub async fn poll_until<F, Fut>(mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..80 {
        if condition().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    condition().await
}

pub struct ControlledStartupStore {
    pub summaries: Vec<SessionSummary>,
    snapshots: HashMap<SessionId, SessionSnapshot>,
    session_gates: Mutex<HashMap<SessionId, Arc<tokio::sync::Semaphore>>>,
    tree_summary_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    failed_summaries: AtomicBool,
    failed_session_ids: Mutex<HashSet<SessionId>>,
    pub requested_session_ids: Mutex<Vec<SessionId>>,
    pub load_calls: AtomicUsize,
    pub save_calls: AtomicUsize,
    pub unarchived_summary_calls: AtomicUsize,
    pub all_summary_calls: AtomicUsize,
}

impl ControlledStartupStore {
    pub fn new(entries: &[(SessionId, jiff::Timestamp)]) -> Self {
        let snapshots = entries
            .iter()
            .map(|(session_id, updated_at)| {
                let mut session = ChatSessionState::new();
                session.set_session_id(session_id.clone());
                session.restore_updated_at(*updated_at);
                (session_id.clone(), session.capture_snapshot())
            })
            .collect();
        let summaries = entries
            .iter()
            .map(|(session_id, updated_at)| SessionSummary {
                session_id: session_id.clone(),
                title: session_id.to_string(),
                updated_at: *updated_at,
                created_at: jiff::Timestamp::UNIX_EPOCH,
                session_state: SessionState::Loaded,
                parent_session: None,
                project: None,
            })
            .collect();
        Self {
            summaries,
            snapshots,
            session_gates: Mutex::new(HashMap::new()),
            tree_summary_gate: Mutex::new(None),
            failed_summaries: AtomicBool::new(false),
            failed_session_ids: Mutex::new(HashSet::new()),
            requested_session_ids: Mutex::new(Vec::new()),
            load_calls: AtomicUsize::new(0),
            save_calls: AtomicUsize::new(0),
            unarchived_summary_calls: AtomicUsize::new(0),
            all_summary_calls: AtomicUsize::new(0),
        }
    }

    pub fn fail_summaries(&self) {
        self.failed_summaries.store(true, Ordering::SeqCst);
    }

    pub fn fail_session(&self, session_id: SessionId) {
        self.failed_session_ids
            .lock()
            .expect("failed session IDs")
            .insert(session_id);
    }

    pub fn gate_session_load(&self, session_id: SessionId) {
        self.session_gates
            .lock()
            .expect("session gates")
            .insert(session_id, Arc::new(tokio::sync::Semaphore::new(0)));
    }

    pub fn release_session_load(&self, session_id: &SessionId) {
        self.session_gates
            .lock()
            .expect("session gates")
            .get(session_id)
            .expect("session load gate")
            .add_permits(1);
    }

    pub async fn wait_for_session_load(&self, session_id: &SessionId) {
        let observed = poll_until(|| async {
            self.requested_session_ids
                .lock()
                .expect("requested session IDs")
                .contains(session_id)
        })
        .await;
        assert!(observed, "session load should be requested");
    }

    pub fn gate_tree_summary_load(&self) {
        *self.tree_summary_gate.lock().expect("tree summary gate") =
            Some(Arc::new(tokio::sync::Semaphore::new(0)));
    }

    pub async fn wait_for_tree_summary_load(&self) {
        let observed =
            poll_until(|| async { self.all_summary_calls.load(Ordering::SeqCst) > 0 }).await;
        assert!(observed, "tree summary load should be requested");
    }
}

#[async_trait]
impl SessionStore for ControlledStartupStore {
    fn name(&self) -> &'static str {
        "controlled-startup"
    }

    async fn save(&self, _snapshot: &SessionSnapshot) -> Result<(), Report<SessionStoreError>> {
        self.save_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        self.all_summary_calls.fetch_add(1, Ordering::SeqCst);
        let gate = self
            .tree_summary_gate
            .lock()
            .expect("tree summary gate")
            .clone();
        if let Some(gate) = gate {
            gate.acquire()
                .await
                .expect("tree summary load gate open")
                .forget();
        }
        if self.failed_summaries.load(Ordering::SeqCst) {
            return Err(Report::new(SessionStoreError));
        }
        Ok(self.summaries.clone())
    }

    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionSnapshot>, Report<SessionStoreError>> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        self.requested_session_ids
            .lock()
            .expect("requested session IDs")
            .push(session_id.clone());
        let gate = self
            .session_gates
            .lock()
            .expect("session gates")
            .get(session_id)
            .cloned();
        if let Some(gate) = gate {
            gate.acquire()
                .await
                .expect("session load gate open")
                .forget();
        }
        if self
            .failed_session_ids
            .lock()
            .expect("failed session IDs")
            .contains(session_id)
        {
            return Err(Report::new(SessionStoreError));
        }
        Ok(self.snapshots.get(session_id).cloned())
    }

    async fn delete(&self, _session_id: &SessionId) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn fork(
        &self,
        _source_session_id: &SessionId,
        _at_ordinal: usize,
    ) -> Result<SessionId, Report<SessionStoreError>> {
        Ok(SessionId::new())
    }

    async fn set_archived(
        &self,
        _session_id: &SessionId,
        _archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn set_archived_many(
        &self,
        _session_ids: &[SessionId],
        _archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        self.unarchived_summary_calls.fetch_add(1, Ordering::SeqCst);
        if self.failed_summaries.load(Ordering::SeqCst) {
            return Err(Report::new(SessionStoreError));
        }
        Ok(self.summaries.clone())
    }

    async fn dirty_session_ids(&self) -> Result<Vec<SessionId>, Report<SessionStoreError>> {
        Ok(Vec::new())
    }

    async fn reindex_session_chunk(
        &self,
        _session_id: &SessionId,
        _max_entries: usize,
    ) -> Result<bool, Report<SessionStoreError>> {
        Ok(true)
    }

    async fn pending_dirty_count(&self) -> Result<usize, Report<SessionStoreError>> {
        Ok(0)
    }

    async fn search(
        &self,
        _params: SearchParams,
    ) -> Result<SearchOutcome, Report<SessionStoreError>> {
        Ok(SearchOutcome {
            total_matches: 0,
            per_session: Vec::new(),
            hits: Vec::new(),
        })
    }

    async fn fetch_window(
        &self,
        _session_id: &SessionId,
        _anchor: &ChatEntryId,
        _context: usize,
    ) -> Result<Option<TranscriptWindow>, Report<SessionStoreError>> {
        Ok(None)
    }

    async fn fetch_tail(
        &self,
        _session_id: &SessionId,
        _limit: usize,
    ) -> Result<Option<TranscriptWindow>, Report<SessionStoreError>> {
        Ok(None)
    }
}
