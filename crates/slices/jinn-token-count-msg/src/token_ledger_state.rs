//! The per-session token ledger, as a slice cell.
//!
//! The ledger used to live inside `ChatSessionState` as a bare `Vec`.
//! It is now a cell here — keyed by session id, like the chat log's
//! per-session view state — so a slice that reads the ledger resolves it
//! the same way it resolves every other cell.
//!
//! Readers must not grow the map (a session with no entry reads as an
//! empty ledger); only writers get-or-insert.

use std::collections::HashMap;

use jinn_core_types::SessionId;
use jinn_slices::SlotKey;

use crate::token_stats::TokenRecord;

/// The token-count cell's ledger payload: one ledger per session.
pub type TokenLedgers = HashMap<SessionId, Vec<TokenRecord>>;

/// The slot key the per-session token ledger lives under.
#[must_use]
pub fn token_ledgers_slot() -> SlotKey {
    SlotKey::builtin("token-count", "ledger")
}
