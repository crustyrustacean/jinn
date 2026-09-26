//! Minimal session data needed to resolve a visible session-list row.

use std::collections::HashMap;

use jinn_core_types::SessionId;

/// Identity and tree position for one loaded session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTreeNode {
    /// Session identity.
    pub id: SessionId,
    /// Creation time used for visible-tree ordering.
    pub created_at: jiff::Timestamp,
    /// Persisted direct parent, when the session was forked or spawned.
    pub parent_id: Option<SessionId>,
}

/// Resolves the loaded session at one visible-tree row index.
#[must_use]
pub fn visible_session_at(
    nodes: Vec<SessionTreeNode>,
    visual_parents: &HashMap<SessionId, SessionId>,
    index: usize,
) -> Option<SessionId> {
    let entries = nodes
        .into_iter()
        .map(|node| crate::SessionEntry {
            kind: crate::SessionEntryKind::Session,
            id: node.id,
            title: String::new(),
            is_active: false,
            created_at: node.created_at,
            is_idle: true,
            last_entry_is_error: false,
            parent_id: node.parent_id,
            depth: 0,
            ancestor_continuations: vec![],
            is_last_child: false,
            is_subagent: false,
            has_live_term: false,
            is_in_flight: false,
        })
        .collect();
    crate::visible_session_tree(entries, visual_parents)
        .get(index)
        .map(|entry| entry.id.clone())
}
