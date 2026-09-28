//! Discord's connection cell: the payload the status actor folds and every
//! feature gate reads.
//!
//! The state and its slot key live here rather than in the slice crate so
//! the shared cell catalog can register the slot without depending on
//! `jinn-discord`, which depends on the kernel. The slice re-exports both
//! so its own modules keep reading them from one path.

use jinn_slices::SlotKey;

/// Discord's own connection fact, folded by the slice's status actor.
///
/// The single source of truth for "is the bot connected": feature gates
/// (e.g. thread creation) read this cell instead of greping the
/// dashboard's actor table. One writer — the status actor's fold.
#[derive(Debug, Clone)]
pub struct ConnectionState {
    /// Whether the gateway considers the bot online.
    pub connected: bool,
    /// Optional detail (e.g. the error message while disconnected).
    pub detail: Option<String>,
}

impl Default for ConnectionState {
    /// A gateway that has not reported yet is not connected.
    fn default() -> Self {
        Self {
            connected: false,
            detail: None,
        }
    }
}

/// Discord's connection cell slot in the slices registry.
///
/// Canonical key shared by the cell catalog (which mints the cell), the
/// status actor (which folds it), and feature gates (which read it).
#[must_use]
pub fn discord_connection_slot() -> SlotKey {
    SlotKey::builtin("discord", "connection")
}
