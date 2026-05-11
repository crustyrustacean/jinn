//! Service wrapper for session storage.
//!
//! Wraps `Arc<dyn SessionStore>` for shared ownership across the application.
//! Follows the service wrapper pattern from the project style guide.

use std::sync::Arc;

use error_stack::Report;

use crate::protocol::SessionId;
use crate::feat::session::{PersistedSession, SessionSummary};

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

    /// Append a session snapshot to the store.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the write fails.
    pub fn save(&self, session: &PersistedSession) -> Result<(), Report<SessionStoreError>> {
        self.svc.save(session)
    }

    /// Scan all lines and return lightweight summaries with byte offsets.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the file cannot be opened or read.
    pub fn load_summaries(
        &self,
    ) -> Result<Vec<(SessionId, SessionSummary, u64)>, Report<SessionStoreError>> {
        self.svc.load_summaries()
    }

    /// Load a full session by seeking to the given byte offset.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the seek or read fails.
    pub fn load_full(
        &self,
        byte_offset: u64,
    ) -> Result<Option<PersistedSession>, Report<SessionStoreError>> {
        self.svc.load_full(byte_offset)
    }

    /// Rewrite the store, keeping only the latest snapshot per session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the rewrite fails.
    pub fn compact(&self) -> Result<(), Report<SessionStoreError>> {
        self.svc.compact()
    }
}
