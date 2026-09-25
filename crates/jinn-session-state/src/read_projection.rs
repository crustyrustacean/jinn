//! Immutable value projections for read-only session consumers.

use jinn_core_types::ChatEntry;
use jinn_session_msg::PhaseKind;

use crate::ChatSessionState;

/// Coherent read-only projection of the session fields needed by external consumers.
///
/// The projection is captured while the application read guard is held, so its
/// history and phase come from one authoritative state version. It exposes no
/// mutable path back into [`ChatSessionState`].
#[derive(Debug, Clone)]
pub struct SessionReadProjection {
    entries: Vec<ChatEntry>,
    phase: PhaseKind,
}

impl SessionReadProjection {
    /// Returns the ordered persisted conversation entries.
    #[must_use]
    pub fn entries(&self) -> &[ChatEntry] {
        &self.entries
    }

    /// Returns the current session phase.
    #[must_use]
    pub fn phase(&self) -> PhaseKind {
        self.phase
    }
}

impl From<&ChatSessionState> for SessionReadProjection {
    fn from(session: &ChatSessionState) -> Self {
        Self {
            entries: session.history().to_vec(),
            phase: session.phase(),
        }
    }
}
