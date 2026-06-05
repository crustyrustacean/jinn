//! State types for the sessions sidebar section.

use std::collections::{HashMap, HashSet};

use crate::common::app_state::AppState;
use crate::feat::session::phase_machine::PhaseKind;
use crate::protocol::SessionId;

/// Discriminator for sidebar list entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionEntryKind {
    Session,
    Plugin { enabled: bool },
}

/// Sessions section cursor state - stored on `FrontendState`.
///
/// Tracks the selected index within the sorted open sessions list.
/// `None` means no cursor (section not focused).
#[derive(Debug, Clone, Default)]
pub struct SessionsSectionState {
    /// Index into the sorted open sessions list.
    pub selected_index: Option<usize>,
    /// Scroll offset: the first session entry index that is visible.
    pub scroll_offset: usize,
    /// Visual-parent index: maps a loaded session to its nearest loaded ancestor
    /// when the direct parent has been archived/removed from memory.
    /// Updated reactively in `remove_and_replace()`, invalidated on session load.
    /// Empty when no intermediate parents have been hidden.
    pub visual_parents: HashMap<SessionId, SessionId>,
}

#[derive(Clone)]
pub(crate) struct SessionEntry {
    /// Whether this entry represents a session or a workflow.
    pub(crate) kind: SessionEntryKind,
    pub(crate) id: SessionId,
    pub(crate) title: String,
    pub(crate) is_active: bool,
    pub(crate) created_at: jiff::Timestamp,
    pub(crate) is_idle: bool,
    pub(crate) last_entry_is_error: bool,

    /// Parent session ID - `None` for root sessions.
    pub(crate) parent_id: Option<SessionId>,
    /// Depth in the session tree. 0 for roots, 1 for their children, etc.
    pub(crate) depth: usize,
    /// For each ancestor level (0..depth-1), `true` if that ancestor has younger siblings.
    /// Used to render `│` vs ` ` continuation characters.
    pub(crate) ancestor_continuations: Vec<bool>,
    /// Whether this entry is the last child of its parent.
    /// Used to render `└` vs `├`.
    pub(crate) is_last_child: bool,
}

/// Collects all loaded sessions in tree order (DFS).
///
/// Roots are sorted by `created_at` descending (newest first).
/// Children under each parent are sorted by `created_at` ascending
/// (oldest child first - they were forked first). Orphaned sessions
/// (parent not in loaded sessions) are treated as roots.
///
/// Only includes sessions with `SessionState::Loaded` - archived sessions
/// are not in the `SessionMap` and thus excluded automatically.
pub(crate) fn sorted_open_sessions(state: &AppState) -> Vec<SessionEntry> {
    let active_id = state.session.active_session_id();

    // Collect all loaded sessions into entries.
    let entries: Vec<SessionEntry> = state
        .session
        .iter()
        .filter(|(_, session)| {
            session.session_state() == crate::feat::session::chat_session::SessionState::Loaded
                && !session.is_workflow()
        })
        .map(|(id, session)| SessionEntry {
            kind: SessionEntryKind::Session,
            id: id.clone(),
            title: session.title().unwrap_or("Untitled Session").to_owned(),
            is_active: id == active_id,
            created_at: *session.created_at(),
            is_idle: matches!(session.phase(), PhaseKind::Idle) && !session.is_busy(),
            last_entry_is_error: session
                .history()
                .last()
                .is_some_and(|e| matches!(&e.kind, crate::protocol::ChatEntryKind::Error(..))),
            parent_id: session.parent_session().clone(),
            depth: 0,
            ancestor_continuations: vec![],
            is_last_child: false,
        })
        .collect();

    // Index entries by ID for O(1) lookup.
    let mut entry_map: HashMap<SessionId, SessionEntry> =
        entries.into_iter().map(|e| (e.id.clone(), e)).collect();

    // Build parent → children map and identify roots.
    // Uses visual_parents index to resolve effective parent when
    // the direct parent has been archived/removed from memory.
    let visual_parents = &state.frontend.sessions_section.visual_parents;
    let mut children_map: HashMap<SessionId, Vec<SessionId>> = HashMap::new();
    let mut roots: Vec<SessionId> = Vec::new();

    // Track effective parent for each entry so we can patch entry.parent_id later.
    let mut effective_parents: HashMap<SessionId, SessionId> = HashMap::new();

    for entry in entry_map.values() {
        let effective_parent = match &entry.parent_id {
            // Direct parent is loaded - use it directly.
            Some(pid) if entry_map.contains_key(pid) => Some(pid.clone()),
            // Direct parent not loaded - try visual_parents for reparenting.
            Some(pid) => visual_parents
                .get(pid)
                .or_else(|| visual_parents.get(&entry.id))
                .filter(|vp| entry_map.contains_key(*vp))
                .cloned(),
            // No parent at all.
            None => None,
        };

        match effective_parent {
            Some(ref pid) => {
                children_map
                    .entry(pid.clone())
                    .or_default()
                    .push(entry.id.clone());
                effective_parents.insert(entry.id.clone(), pid.clone());
            }
            None => {
                roots.push(entry.id.clone());
            }
        }
    }

    // Patch entry.parent_id to reflect effective visual parent.
    for (id, ep) in effective_parents {
        if let Some(entry) = entry_map.get_mut(&id) {
            entry.parent_id = Some(ep);
        }
    }

    // Sort roots descending by created_at (newest first).
    roots.sort_by(|a, b| entry_map[b].created_at.cmp(&entry_map[a].created_at));

    // Sort each parent's children ascending by created_at (oldest first).
    for children in children_map.values_mut() {
        children.sort_by(|a, b| entry_map[a].created_at.cmp(&entry_map[b].created_at));
    }

    // DFS traversal to produce flat list with tree metadata.
    let mut result: Vec<SessionEntry> = Vec::new();
    let mut visited: HashSet<SessionId> = HashSet::new();
    let root_count = roots.len();

    for (i, root_id) in roots.iter().enumerate() {
        if !visited.insert(root_id.clone()) {
            continue;
        }
        let Some(mut entry) = entry_map.get(root_id).cloned() else {
            continue;
        };
        entry.depth = 0;
        entry.ancestor_continuations = vec![];
        entry.is_last_child = i == root_count - 1;
        result.push(entry);
        dfs_children(
            root_id,
            &children_map,
            &entry_map,
            &mut result,
            vec![],
            &mut visited,
            i == root_count - 1,
        );
    }

    // Post-DFS pass: insert workflow child entries under each session.
    // For each Session entry, find where its subtree ends, then insert
    // workflow entries for each attached workflow.
    insert_workflow_entries(state, &mut result);

    result
}
/// Post-DFS pass that inserts workflow child entries under their parent sessions.
///
/// After the DFS builds the session tree, this function scans the result list
/// and for each `Session` entry, looks up its `attached_plugins` from state.
/// Plugin entries are inserted after the session's last real child (or immediately
/// after the session if it has no children). Tree metadata (depth, continuations,
/// `is_last_child`) is computed to maintain visual consistency.
///
/// Sessions that previously were the last child of *their* parent but now have
/// workflow children appended need their own `is_last_child` unchanged (they remain
/// last among *session* siblings). The workflow entries become their new children at
/// depth + 1.
fn insert_workflow_entries(state: &AppState, entries: &mut Vec<SessionEntry>) {
    // Collect (insert_index, session_id, workflows) for each session with workflows.
    // We scan the result list and find the boundary where each session's subtree ends.
    let mut insertions: Vec<(
        usize,
        SessionId,
        Vec<crate::feat::attached_plugin::AttachedPlugin>,
    )> = Vec::new();

    let mut i = 0;
    while i < entries.len() {
        let entry = &entries[i];
        if !matches!(entry.kind, SessionEntryKind::Session) {
            i += 1;
            continue;
        }

        let session_id = entry.id.clone();
        let session_depth = entry.depth;

        // Find where this session's subtree ends.
        // The subtree includes the session itself and all descendants (depth > session_depth).
        let subtree_end = entries[i + 1..]
            .iter()
            .position(|e| e.depth <= session_depth)
            .map_or(entries.len(), |p| i + 1 + p);

        // Look up attached workflows for this session.
        let Some(session) = state.session.get(&session_id) else {
            i = subtree_end;
            continue;
        };

        let workflows = session.core.attached_plugins.clone();
        if !workflows.is_empty() {
            insertions.push((subtree_end, session_id, workflows));
        }

        i = subtree_end;
    }

    // Insert workflow entries in reverse order so indices remain valid.
    for (insert_idx, parent_id, workflows) in insertions.into_iter().rev() {
        let parent_entry = entries
            .iter()
            .find(|e| matches!(e.kind, SessionEntryKind::Session) && e.id == parent_id);
        let Some(parent) = parent_entry else { continue };
        let parent_depth = parent.depth;
        let parent_continuations = parent.ancestor_continuations.clone();
        let parent_created_at = parent.created_at;

        // Build the ancestor_continuations for workflow children: parent's continuations + whether parent has younger siblings.
        // Since workflows are appended *after* all real children, the parent effectively
        // has workflow children — but the continuation line depends on whether the parent
        // is the last child of *its* parent.
        let workflow_depth = parent_depth + 1;
        let mut workflow_continuations = parent_continuations;
        // The parent's is_last_child determines if we draw │ or space at the parent level.
        // But we need to know if the parent has younger session siblings. If parent.is_last_child, no continuation.
        // If not, draw │.
        // However, the parent's own is_last_child may need re-evaluation if we add workflow children
        // after the last real child. Actually, the parent's is_last_child refers to its position among
        // its *session* siblings, which is unchanged by adding workflow children.
        let parent_continues = !parent.is_last_child;
        workflow_continuations.push(parent_continues);

        let wf_count = workflows.len();
        for (j, ap) in workflows.into_iter().enumerate() {
            let is_last = j == wf_count - 1;
            let wf_entry = SessionEntry {
                kind: SessionEntryKind::Plugin {
                    enabled: ap.enabled,
                },
                id: parent_id.clone(),
                title: ap.label_or_name().to_owned(),
                is_active: false,
                created_at: parent_created_at,
                is_idle: true,
                last_entry_is_error: false,
                parent_id: Some(parent_id.clone()),
                depth: workflow_depth,
                ancestor_continuations: workflow_continuations.clone(),
                is_last_child: is_last,
            };
            entries.insert(insert_idx + j, wf_entry);
        }

        // Fix is_last_child of the parent's last *real* child if the parent previously had children.
        // The last real child might have had is_last_child = true, but now workflow entries come after it.
        // We need to set it to false so it renders as ├─ instead of └─.
        // Walk backwards from insert_idx to find the last entry that belongs to this parent's subtree
        // and has depth == workflow_depth.
        if insert_idx > 0 {
            for k in (0..insert_idx).rev() {
                if entries[k].depth == workflow_depth
                    && entries[k].parent_id.as_ref() == Some(&parent_id)
                    && matches!(entries[k].kind, SessionEntryKind::Session)
                {
                    entries[k].is_last_child = false;
                    break;
                }
            }
        }
    }
}

/// Updates the visual-parent index when a session is about to be removed.
///
/// Must be called BEFORE the session is removed from the `SessionMap`.
/// Resolves the nearest loaded ancestor for any orphaned children and
/// updates the `visual_parents` index accordingly.
///
/// Also handles transitive chains: if any existing `visual_parents` entries
/// point to the removed session as their effective ancestor, those entries
/// are updated to point to the resolved ancestor instead.
pub fn update_visual_parents_on_removal(state: &mut AppState, removed_id: &SessionId) {
    let effective_ancestor = {
        // Resolve the nearest loaded ancestor for the session being removed.
        let Some(removed_session) = state.session.get(removed_id) else {
            return;
        };

        match removed_session.parent_session() {
            // Direct parent is loaded - use it.
            Some(pid) if state.session.contains(pid) => Some(pid.clone()),
            // Direct parent not loaded - check if it has a visual_parents entry.
            Some(pid) => state
                .frontend
                .sessions_section
                .visual_parents
                .get(pid)
                .cloned()
                .or_else(|| {
                    // The parent's parent may not be in visual_parents,
                    // but the removed session itself might have been reparented.
                    state
                        .frontend
                        .sessions_section
                        .visual_parents
                        .get(removed_id)
                        .cloned()
                }),
            // No parent at all - check if the removed session itself has a visual parent.
            None => state
                .frontend
                .sessions_section
                .visual_parents
                .get(removed_id)
                .cloned(),
        }
    };

    let visual_parents = &mut state.frontend.sessions_section.visual_parents;

    // Find direct children of the removed session and reparent them.
    let orphan_ids: Vec<SessionId> = state
        .session
        .iter()
        .filter(|(_, s)| s.parent_session().as_ref() == Some(removed_id))
        .map(|(id, _)| id.clone())
        .collect();

    for orphan_id in orphan_ids {
        match &effective_ancestor {
            Some(ancestor_id) => {
                visual_parents.insert(orphan_id, ancestor_id.clone());
            }
            None => {
                visual_parents.remove(&orphan_id);
            }
        }
    }

    // Update transitive entries: any session already bypassing the removed session.
    let keys_to_update: Vec<SessionId> = visual_parents
        .iter()
        .filter(|(_, v)| *v == removed_id)
        .map(|(k, _)| k.clone())
        .collect();

    for key in keys_to_update {
        match &effective_ancestor {
            Some(ancestor_id) => {
                visual_parents.insert(key, ancestor_id.clone());
            }
            None => {
                visual_parents.remove(&key);
            }
        }
    }
}

/// Removes stale `visual_parents` entries when a session is loaded back.
///
/// When a session is unarchived/loaded, its children no longer need to bypass
/// it in the tree. This function removes any `visual_parents` entries whose
/// **value** equals the loaded session's ID.
///
/// Entries where the loaded session is the **key** are preserved - the loaded
/// session may itself have a hidden parent that it needs to be reparented under.
pub fn clear_visual_parents_on_load(state: &mut AppState, loaded_id: &SessionId) {
    state
        .frontend
        .sessions_section
        .visual_parents
        .retain(|_k, v| v != loaded_id);
}

/// Recursively appends children of `parent_id` to `result` in DFS order.
///
/// `ancestor_continuations` tracks whether each ancestor level has younger
/// siblings - used to draw `│` continuation lines.
fn dfs_children(
    parent_id: &SessionId,
    children_map: &HashMap<SessionId, Vec<SessionId>>,
    entry_map: &HashMap<SessionId, SessionEntry>,
    result: &mut Vec<SessionEntry>,
    ancestor_continuations: Vec<bool>,
    visited: &mut HashSet<SessionId>,
    parent_is_last: bool,
) {
    let children = children_map.get(parent_id).cloned().unwrap_or_default();

    if children.is_empty() {
        return;
    }

    // Determine continuation lines.
    let mut continuations = ancestor_continuations;
    continuations.push(!parent_is_last);

    for (i, child_id) in children.iter().enumerate() {
        if !visited.insert(child_id.clone()) {
            continue; // cycle guard
        }
        let is_last = i == children.len() - 1;
        let Some(mut entry) = entry_map.get(child_id).cloned() else {
            continue;
        };
        entry.depth = continuations.len();
        entry.ancestor_continuations.clone_from(&continuations);
        entry.is_last_child = is_last;
        result.push(entry);
        dfs_children(
            child_id,
            children_map,
            entry_map,
            result,
            continuations.clone(),
            visited,
            is_last,
        );
    }
}
