//! Picker kind — identifies which picker is currently active
//! (shared vocabulary; this is the canonical home — the kernel re-exports it via `jinn_domain::protocol`).

use serde::{Deserialize, Serialize};

/// Which picker is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickerKind {
    /// Session lifecycle picker - select a lifecycle recipe for new session creation.
    /// Retired: the compaction model is configured only by `[compaction] model`
    /// in `jinn.toml`. Retained so persisted state still deserializes; it is
    /// never pushed as a scope and has no picker state.
    CompactionModel,
    /// Project picker - curated project directories; create a new session rooted
    /// at the highlighted dir with `<enter>` (or `<c-enter>` to also pick a lifecycle).
    Project,
    /// MCP server picker - toggle which MCP servers are enabled for the session.
    McpServer,
}

impl std::fmt::Display for PickerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CompactionModel => write!(f, "compaction model"),

            Self::Project => write!(f, "projects"),
            Self::McpServer => write!(f, "mcp servers"),
        }
    }
}
