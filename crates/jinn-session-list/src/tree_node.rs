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
    visible_rows(nodes, visual_parents)
        .get(index)
        .map(|entry| entry.id.clone())
}

/// The visible-tree row index at which `id` is drawn, or `None` when it is
/// not drawn.
///
/// The inverse of [`visible_session_at`], and the only supported way to ask
/// where a session lands: a caller holding an identity — because the thing it
/// acts on is an identity — still needs the row to draw a cursor band on, to
/// anchor a popup to, or to scroll a row into view.
///
/// Both directions project through [`visible_rows`], so they cannot disagree
/// about what the list looks like. Resolving the index by hand — filtering,
/// re-deriving, counting visible rows by hand — is what produced a cursor
/// that named the wrong session when the two lists drifted apart.
#[must_use]
pub fn visible_index_of(
    nodes: Vec<SessionTreeNode>,
    visual_parents: &HashMap<SessionId, SessionId>,
    id: &SessionId,
) -> Option<usize> {
    visible_rows(nodes, visual_parents)
        .iter()
        .position(|entry| &entry.id == id)
}

/// The visible tree as rows, built from identity alone.
///
/// Titles and the display-only flags are placeholders: this exists to answer
/// "in what order, and where", so it deliberately does not clone what the
/// caller already has.
fn visible_rows(
    nodes: Vec<SessionTreeNode>,
    visual_parents: &HashMap<SessionId, SessionId>,
) -> Vec<crate::SessionEntry> {
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
            is_attendant: false,
            is_attendant_prepping: false,
            attendant_fires_on_parent_completion: false,
            has_live_term: false,
            is_in_flight: false,
        })
        .collect::<Vec<_>>();

    crate::visible_session_tree(entries, visual_parents)
}
