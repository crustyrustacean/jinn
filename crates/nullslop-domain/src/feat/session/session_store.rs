//! Session store abstraction and SQLite implementation.
//!
//! Defines [`SessionStore`] as the async trait for session persistence and
//! [`SqliteSessionStore`] as the SQLite-backed implementation. Sessions are
//! stored in normalized tables with a junction table for entries, enabling
//! fork support without data duplication.

mod migrator;
mod service;
mod sqlite;
#[cfg(test)]
mod sqlite_tests;

pub use service::SessionStoreService;
pub use sqlite::{PoolConfig, SqliteSessionStore};

use async_trait::async_trait;
use error_stack::Report;
use wherror::Error;

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::session_summary::SessionSummary;
use crate::protocol::SessionId;

/// Error type for session store operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct SessionStoreError;

/// Abstraction for session persistence.
///
/// Every external dependency must have a trait abstraction (AGENTS.md §2).
/// SQLite I/O is an external dependency — this trait abstracts it so
/// tests can swap in-memory storage.
///
/// All methods are async. Implementations use `tokio::task::spawn_blocking`
/// to bridge synchronous SQLite calls into the async runtime.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// Returns the storage backend name (for debugging).
    fn name(&self) -> &'static str;

    /// Save a complete session.
    ///
    /// Upserts session metadata, entries, and token ledger in one transaction.
    /// Entries are deduplicated across sessions via the junction table.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the write fails.
    async fn save(&self, session: &ChatSessionState) -> Result<(), Report<SessionStoreError>>;

    /// Load lightweight summaries for all sessions.
    ///
    /// Returns one [`SessionSummary`] per session, suitable for picker display.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the database cannot be read.
    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>>;

    /// Load a full session by ID.
    ///
    /// Returns `None` if no session with the given ID exists.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the read fails.
    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<ChatSessionState>, Report<SessionStoreError>>;

    /// Delete a session and all its data.
    ///
    /// Removes the session row, its junction rows, and any orphaned entries
    /// (entries no longer referenced by any session). Token ledger rows for
    /// the session are also deleted via `ON DELETE CASCADE`.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the delete fails.
    async fn delete(&self, session_id: &SessionId) -> Result<(), Report<SessionStoreError>>;

    /// Fork a session from a specific entry ordinal into a new session.
    ///
    /// Creates a new session with `parent_session` = `source_session_id`.
    /// Copies junction rows from the source session for entries with
    /// ordinal <= `at_ordinal`. Entry data is shared, not duplicated.
    /// The new session gets its own independent token ledger.
    ///
    /// Returns the new session's ID.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the source session doesn't exist or
    /// the fork fails.
    async fn fork(
        &self,
        source_session_id: &SessionId,
        at_ordinal: usize,
    ) -> Result<SessionId, Report<SessionStoreError>>;

    /// Set the `archived` flag for a session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the update fails.
    async fn set_archived(
        &self,
        session_id: &SessionId,
        archived: bool,
    ) -> Result<(), Report<SessionStoreError>>;

    /// Load lightweight summaries for all unarchived sessions.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the database cannot be read.
    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<SessionSummary>, Report<SessionStoreError>>;

    /// Shut down the store, performing any cleanup or flush operations.
    ///
    /// Called once during application shutdown. Implementations may use
    /// this to delete stale data, fsync, flush WAL, etc. The default
    /// implementation is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the cleanup fails.
    async fn shutdown(&self) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }
}

impl std::fmt::Debug for dyn SessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStore")
            .field("name", &self.name())
            .finish()
    }
}
