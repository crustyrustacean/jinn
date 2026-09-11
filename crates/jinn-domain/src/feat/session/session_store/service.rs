//! Service wrapper for session storage.
//!
//! Wraps `Arc<dyn SessionStore>` for shared ownership across the application.
//! Follows the service wrapper pattern from the project style guide.

use std::sync::Arc;

use error_stack::Report;

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::session_summary::SessionSummary;
use crate::feat::session_search::{SearchOutcome, SearchParams, TranscriptWindow};
use crate::protocol::{ChatEntryId, SessionId};

use super::{SessionStore, SessionStoreError};

/// Service wrapper for session storage.
///
/// Wraps `Arc<dyn SessionStore>` for shared ownership across the application.
/// Follows the service wrapper pattern from the project style guide.
#[derive(Debug, Clone)]
pub struct SessionStoreService {
    /// The underlying session store implementation.
    svc: Arc<dyn SessionStore>,
}

impl SessionStoreService {
    /// Creates a new session store service.
    #[must_use]
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { svc: store }
    }

    /// Save a complete session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the write fails.
    pub async fn save(&self, session: &ChatSessionState) -> Result<(), Report<SessionStoreError>> {
        self.svc.save(session).await
    }

    /// Load lightweight summaries for all sessions.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the database cannot be read.
    pub async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        self.svc.load_summaries().await
    }

    /// Load a full session by ID.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the read fails.
    pub async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
        self.svc.load_session(session_id).await
    }

    /// Delete a session and all its data.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the delete fails.
    pub async fn delete(&self, session_id: &SessionId) -> Result<(), Report<SessionStoreError>> {
        self.svc.delete(session_id).await
    }

    /// Fork a session from a specific entry ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the source session doesn't exist or
    /// the fork fails.
    pub async fn fork(
        &self,
        source_session_id: &SessionId,
        at_ordinal: usize,
    ) -> Result<SessionId, Report<SessionStoreError>> {
        self.svc.fork(source_session_id, at_ordinal).await
    }

    /// Set the `archived` flag for a session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the update fails.
    pub async fn set_archived(
        &self,
        session_id: &SessionId,
        archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        self.svc.set_archived(session_id, archived).await
    }

    /// Set the `archived` flag for many sessions in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the update fails.
    pub async fn set_archived_many(
        &self,
        session_ids: &[SessionId],
        archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        self.svc.set_archived_many(session_ids, archived).await
    }

    /// Load lightweight summaries for all unarchived sessions.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the database cannot be read.
    pub async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        self.svc.load_unarchived_summaries().await
    }

    /// Returns the ids of all sessions with pending (dirty) FTS reindex work.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the read fails.
    pub async fn dirty_session_ids(&self) -> Result<Vec<SessionId>, Report<SessionStoreError>> {
        self.svc.dirty_session_ids().await
    }

    /// Recomputes one session's FTS rows and clears its dirty marker
    /// (search index maintenance).
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if any read or write fails.
    pub async fn reindex_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), Report<SessionStoreError>> {
        self.svc.reindex_session(session_id).await
    }

    /// Returns how many sessions have pending (dirty) FTS reindex work.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the read fails.
    pub async fn pending_dirty_count(&self) -> Result<usize, Report<SessionStoreError>> {
        self.svc.pending_dirty_count().await
    }

    /// Run an FTS query over the search index.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the query fails, including FTS5
    /// syntax errors surfaced with the verbatim SQLite message.
    pub async fn search(
        &self,
        params: SearchParams,
    ) -> Result<SearchOutcome, Report<SessionStoreError>> {
        self.svc.search(params).await
    }

    /// Load a window of entries around an anchor entry.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the read fails.
    pub async fn fetch_window(
        &self,
        session_id: &SessionId,
        anchor: &ChatEntryId,
        context: usize,
    ) -> Result<Option<TranscriptWindow>, Report<SessionStoreError>> {
        self.svc.fetch_window(session_id, anchor, context).await
    }

    /// Load the last `limit` entries of a session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the read fails.
    pub async fn fetch_tail(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> Result<Option<TranscriptWindow>, Report<SessionStoreError>> {
        self.svc.fetch_tail(session_id, limit).await
    }

    /// Loads all non-archived judge sessions targeting the given origin.
    ///
    /// Shut down the store, performing any cleanup or flush operations.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the cleanup fails.
    pub async fn shutdown(&self) -> Result<(), Report<SessionStoreError>> {
        self.svc.shutdown().await
    }
}
