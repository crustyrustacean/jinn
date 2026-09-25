//! Authoritative durable and runtime composition for one session.

use jinn_core_types::ChatEntry;
use jinn_token_count_msg::TokenRecord;
use serde::{Deserialize, Serialize};

use crate::fields::{
    SessionHistoryWorkFields, SessionIdentityMetadataFields, SessionIntegrationFields,
    SessionLifecycleLocationFields, SessionStorageFields,
};
use crate::runtime::SessionCoreEphemeral;

/// Coherent session aggregate stored behind one live-state lock.
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
}

impl SessionCore {
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
        }
    }
}
