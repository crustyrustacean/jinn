//! Neutral row model shared by sidebar and other session-list consumers.

use jinn_core_types::SessionId;

/// Discriminator for a sessions-list row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEntryKind {
    /// A loaded session row.
    Session,
}

/// One visible row in a session tree, including render geometry metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    /// The kind of this row.
    pub kind: SessionEntryKind,
    /// Session identity.
    pub id: SessionId,
    /// Display title.
    pub title: String,
    /// Whether this session is active.
    pub is_active: bool,
    /// Creation time used to order roots and children.
    pub created_at: jiff::Timestamp,
    /// Whether the session is idle and not busy.
    pub is_idle: bool,
    /// Whether the final history entry is an error.
    pub last_entry_is_error: bool,
    /// Effective parent after visual-parent repair.
    pub parent_id: Option<SessionId>,
    /// Depth in the visible tree; zero for roots.
    pub depth: usize,
    /// Whether each ancestor level has a continuation line.
    pub ancestor_continuations: Vec<bool>,
    /// Whether this row is the last child of its parent.
    pub is_last_child: bool,
    /// Whether this session is a task-tool subagent.
    pub is_subagent: bool,
    /// Whether the session currently owns a live interactive terminal.
    pub has_live_term: bool,
    /// Whether a disposal operation for this session has been dispatched and
    /// has not yet finished. Rendered as a background wash so a row whose
    /// archive or teardown is still running is distinguishable at a glance.
    pub is_in_flight: bool,
}
