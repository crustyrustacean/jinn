//! The chat log's line cache as a first-class cell payload.
//!
//! The cache used to sit on `AppState` as a bare `RwLock`, which made it the
//! one piece of theme-sensitive rendering state no slice owned. It is now
//! registered as a cell in this crate — the slice that writes it, and the one
//! place a theme change has to reach — so the cell handle is the only lock
//! guarding it and there is no second copy under a second invalidation rule.

use jinn_slices::SlotKey;

/// The slot key the chat log's line cache lives under.
#[must_use]
pub fn entry_line_cache_slot() -> SlotKey {
    SlotKey::builtin("chat-log-view", "line-cache")
}
