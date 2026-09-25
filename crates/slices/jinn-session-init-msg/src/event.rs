//! Events published after session-init resource discovery.

use serde::{Deserialize, Serialize};

use jinn_core_types::SessionId;
use jinn_slices::BusMessage;

/// Emitted when project context files (AGENTS.md/CLAUDE.md) have been scanned
/// and loaded for a session.
///
/// The context-files scan actor emits this after walking the bounded ancestor
/// chain for the session's cwd and reading the first existing candidate per dir.
/// Downstream handlers store the result in the session's ephemeral discovered set.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "Project context files scanned and loaded for a session.")]
pub struct ContextFilesLoaded {
    /// The session whose cwd drove the scan.
    pub session_id: SessionId,
    /// The discovered context files (AGENTS.md / CLAUDE.md), ordered
    /// least-local (root-most ancestor) to most-local (cwd).
    pub files: Vec<jinn_context::ContextFile>,
    /// Error message if scanning failed, `None` on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BusMessage for ContextFilesLoaded {}
