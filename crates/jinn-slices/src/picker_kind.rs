//! Legacy picker scope names, retained only so persisted state still deserializes.
//!
//! Every picker is now slice-owned: it pushes a dynamic slice scope, holds its
//! state in a slice cell, and declares its own keys as route rows. Nothing
//! constructs these variants any more. They survive because a session file
//! saved by an older jinn may still name one, and dropping the variants would
//! make that file fail to load.

use serde::{Deserialize, Serialize};

/// A picker scope name from before pickers became slice-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickerKind {
    /// The compaction model picker, retired long ago: the model is configured
    /// only by `[compaction] model` in `jinn.toml`.
    CompactionModel,
    /// The project picker, now owned by the project slice.
    Project,
    /// The MCP server picker, now owned by the MCP slice.
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
