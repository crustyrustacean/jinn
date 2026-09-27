//! Authoritative durable and runtime composition for one session.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use jinn_core_types::ChatEntry;
use jinn_token_count_msg::TokenRecord;
use serde::{Deserialize, Serialize};

use crate::fields::{
    SessionHistoryWorkFields, SessionIdentityMetadataFields, SessionIntegrationFields,
    SessionLifecycleLocationFields, SessionStorageFields,
};
use crate::runtime::SessionCoreEphemeral;

/// Coherent session aggregate stored behind one live-state lock.
#[expect(
    clippy::partial_pub_fields,
    reason = "durable group fields form the snapshot adapter while the revision counter remains private"
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCore {
    /// Identity and tree metadata.
    #[serde(flatten)]
    pub identity: SessionIdentityMetadataFields,
    /// Location and lifecycle metadata.
    #[serde(flatten)]
    pub lifecycle: SessionLifecycleLocationFields,
    /// History, token, and task state.
    #[serde(flatten)]
    pub history_work: SessionHistoryWorkFields,
    /// Provider and integration state.
    #[serde(flatten)]
    pub integrations: SessionIntegrationFields,
    /// Storage state and policy.
    #[serde(flatten)]
    pub storage: SessionStorageFields,
    /// Runtime-only turn and discovery state.
    #[serde(skip)]
    pub ephemeral: SessionCoreEphemeral,
    /// Monotonic capture sequence shared by clones of this authoritative core.
    #[serde(skip)]
    capture_counter: Arc<AtomicU64>,
}

impl SessionCore {
    /// Resumes capture numbering above a revision storage has already written.
    ///
    /// A core rebuilt from storage starts a fresh counter, so its first
    /// snapshot would carry revision 1 — a number storage has already written
    /// for this session and would refuse. Seeding with the stored revision
    /// makes the next capture [`Self::next_capture_revision`], one past it.
    ///
    /// Seeding never moves a counter backwards: a core that has already
    /// captured past `floor` keeps its own numbering, so a seed applied to a
    /// live session can only ever be a no-op.
    pub fn resume_captures_after(&mut self, floor: crate::snapshot::SessionRevision) {
        let current = self.capture_counter.load(Ordering::Relaxed);
        self.capture_counter
            .store(floor.get().max(current), Ordering::Relaxed);
    }

    /// Reserves the next coherent snapshot revision.
    #[must_use]
    pub fn next_capture_revision(&self) -> crate::snapshot::SessionRevision {
        crate::snapshot::SessionRevision::new(
            self.capture_counter.fetch_add(1, Ordering::Relaxed) + 1,
        )
    }

    /// Restores history before the session is published as live.
    pub fn restore_history(&mut self, entries: Vec<ChatEntry>) {
        self.history_work.history.replace_all(entries);
    }

    /// Restores the token ledger before the session is published as live.
    pub fn restore_token_ledger(&mut self, records: Vec<TokenRecord>) {
        self.history_work.token_ledger = records;
    }
}

impl Default for SessionCore {
    fn default() -> Self {
        Self {
            identity: SessionIdentityMetadataFields::default(),
            lifecycle: SessionLifecycleLocationFields::default(),
            history_work: SessionHistoryWorkFields::default(),
            integrations: SessionIntegrationFields::default(),
            storage: SessionStorageFields::default(),
            ephemeral: SessionCoreEphemeral::default(),
            capture_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}
