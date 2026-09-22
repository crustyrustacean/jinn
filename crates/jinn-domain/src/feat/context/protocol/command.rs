//! Command types for context management.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::BusMessage;
use crate::protocol::SessionId;

/// Load entries for the persona picker.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Load persona picker entries from the persona catalog.")]
pub struct LoadPersonaPickerEntries;

impl BusMessage for LoadPersonaPickerEntries {}

/// Scan project context files (AGENTS.md/CLAUDE.md) for a specific session.
///
/// Carries the session's cwd: the worker walks the bounded ancestor chain
/// (stopping at an exclusive `$HOME` or inclusive VCS root, whichever comes
/// first), reads the first existing candidate per walked dir, and writes the
/// result into that session's ephemeral discovered-context-files set.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Scan project context files for a session.")]
pub struct ScanContextFiles {
    /// The session whose scan this is.
    pub session_id: SessionId,
    /// The working directory driving the scan.
    #[serde(default)]
    pub cwd: PathBuf,
}

impl BusMessage for ScanContextFiles {}
