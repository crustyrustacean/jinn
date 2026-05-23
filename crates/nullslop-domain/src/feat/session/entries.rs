//! Session entries — loading and formatting for the tree-structured picker.
//!
//! Contains loader functions for session picker entries.
//! The [`SessionTreeEntry`] struct and [`TreeItem`] implementation live
//! in `picker_entry.rs`.

use std::collections::HashMap;

use crate::common::app_state::AppState;
use crate::common::services::Services;
use crate::feat::session::picker_entry::SessionTreeEntry;
use crate::feat::theme::Theme;
use crate::protocol::SessionId;

use super::SessionStoreService;

/// Sorts session entries so that whole trees move as a unit.
///
/// Each tree's position is determined by the most recent `updated_at`
/// across all nodes in the tree. Loaded trees appear before Archived trees.
/// Within each tree, children are sorted by `updated_at` descending.
pub(crate) fn sort_entries_tree_aware(entries: &mut Vec<SessionTreeEntry>) {
    if entries.is_empty() {
        return;
    }

    // 1. Index by session_id.
    let id_to_idx: HashMap<SessionId, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.session_id.clone(), i))
        .collect();

    // 2 & 3. Build parent→children map and identify roots.
    let mut children_map: HashMap<SessionId, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();

    for (idx, entry) in entries.iter().enumerate() {
        match &entry.parent_id {
            Some(pid) if id_to_idx.contains_key(pid) => {
                children_map
                    .entry(pid.clone())
                    .or_default()
                    .push(idx);
            }
            _ => {
                roots.push(idx);
            }
        }
    }

    // 4. Compute tree-max updated_at for each root.
    let tree_max: HashMap<SessionId, jiff::Timestamp> = roots
        .iter()
        .map(|&root_idx| {
            let max_ts = compute_subtree_max(root_idx, &children_map, entries);
            (entries[root_idx].session_id.clone(), max_ts)
        })
        .collect();

    // 5 & 6. Sort roots: session_state ascending (Loaded < Archived → Loaded first), then tree-max updated_at descending.
    roots.sort_by(|&a, &b| {
        entries[a]
            .session_state
            .cmp(&entries[b].session_state)
            .then_with(|| tree_max[&entries[b].session_id].cmp(&tree_max[&entries[a].session_id]))
    });

    // 7. Flatten in DFS order, sorting children by updated_at descending.
    let mut sorted = Vec::with_capacity(entries.len());
    for &root_idx in &roots {
        emit_tree(root_idx, &children_map, entries, &mut sorted);
    }
    *entries = sorted;
}

/// Returns the maximum `updated_at` across a node and all its descendants.
fn compute_subtree_max(
    idx: usize,
    children_map: &HashMap<SessionId, Vec<usize>>,
    entries: &[SessionTreeEntry],
) -> jiff::Timestamp {
    let mut max_ts = entries[idx].updated_at;
    if let Some(children) = children_map.get(&entries[idx].session_id) {
        for &child_idx in children {
            let child_max = compute_subtree_max(child_idx, children_map, entries);
            max_ts = max_ts.max(child_max);
        }
    }
    max_ts
}

/// Emits a node and all its descendants in DFS order.
/// Children within each parent are sorted by `updated_at` descending.
fn emit_tree(
    idx: usize,
    children_map: &HashMap<SessionId, Vec<usize>>,
    entries: &[SessionTreeEntry],
    result: &mut Vec<SessionTreeEntry>,
) {
    result.push(entries[idx].clone());
    if let Some(mut children) = children_map.get(&entries[idx].session_id).cloned() {
        children.sort_by(|&a, &b| entries[b].updated_at.cmp(&entries[a].updated_at));
        for &child_idx in &children {
            emit_tree(child_idx, children_map, entries, result);
        }
    }
}

/// Loads session tree entries from the session store, sorted with tree-aware ordering.
///
/// Whole trees move as a unit: each tree's position is determined by the most
/// recent `updated_at` across all nodes in the tree. Loaded trees appear first,
/// followed by archived trees. Within each tree, children are sorted by
/// `updated_at` descending.
/// Errors are logged and result in an empty list.
pub async fn load_session_entries(services: &Services, theme: &Theme) -> Vec<SessionTreeEntry> {
    match services.session_store.load_summaries().await {
        Ok(summaries) => {
            let mut entries: Vec<SessionTreeEntry> = summaries
                .into_iter()
                .map(|summary| {
                    SessionTreeEntry::new(
                        summary.session_id,
                        summary.title,
                        summary.updated_at,
                        theme.clone(),
                        summary.session_state,
                        summary.parent_session,
                    )
                })
                .collect();
            // Tree-aware sort: whole trees move as a unit, positioned by
            // the most recent updated_at in the tree. Loaded first.
            sort_entries_tree_aware(&mut entries);
            entries
        }
        Err(e) => {
            tracing::warn!(err = ?e, "failed to load session summaries");
            vec![]
        }
    }
}

/// Loads session tree entries into the picker state, ready for display.
///
/// Reads from the session store via services and stores the entries via
/// `TreePickerState::set_items`.
pub async fn load_session_picker_items(services: &Services, state: &mut AppState) {
    let entries = load_session_entries(services, &state.frontend.theme).await;
    state.frontend.session_picker.set_items(entries);
}

/// Loads session tree entries from a session store service directly.
///
/// Same as [`load_session_entries`] but accepts the store service directly
/// instead of the full `Services` container.
pub async fn load_session_entries_from_store(
    store: &SessionStoreService,
    theme: &Theme,
) -> Vec<SessionTreeEntry> {
    match store.load_summaries().await {
        Ok(summaries) => {
            let mut entries: Vec<SessionTreeEntry> = summaries
                .into_iter()
                .map(|summary| {
                    SessionTreeEntry::new(
                        summary.session_id,
                        summary.title,
                        summary.updated_at,
                        theme.clone(),
                        summary.session_state,
                        summary.parent_session,
                    )
                })
                .collect();
            // Tree-aware sort: whole trees move as a unit, positioned by
            // the most recent updated_at in the tree. Loaded first.
            sort_entries_tree_aware(&mut entries);
            entries
        }
        Err(e) => {
            tracing::warn!(err = ?e, "failed to load session summaries");
            vec![]
        }
    }
}

/// Loads session tree entries into the picker state from a session store service.
pub async fn load_session_picker_items_from_store(
    store: &SessionStoreService,
    state: &mut AppState,
) {
    let entries = load_session_entries_from_store(store, &state.frontend.theme).await;
    state.frontend.session_picker.set_items(entries);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use crate::feat::session::chat_session::SessionState;
    use crate::feat::session::picker_entry::SessionTreeEntry;
    use crate::feat::theme::default_theme;
    use crate::protocol::SessionId;
    use nullslop_selection_widget::TreeItem;

    use super::*;

    #[rstest::rstest]
    fn session_entry_display_label_returns_title() {
        // Given a SessionTreeEntry with a title.
        let entry = SessionTreeEntry::new(
            SessionId::new(),
            "My Chat".to_owned(),
            jiff::Timestamp::now(),
            default_theme(),
            SessionState::Loaded,
            None,
        );

        // When calling display_label.
        // Then it returns the title.
        assert_eq!(entry.display_label(), "My Chat");
    }

    #[rstest::rstest]
    fn render_row_contains_title() {
        // Given a session tree entry.
        let entry = SessionTreeEntry::new(
            SessionId::new(),
            "My Session".to_owned(),
            jiff::Timestamp::now(),
            default_theme(),
            SessionState::Loaded,
            None,
        );

        // When rendering.
        let row = entry.render_row(false);

        // Then the title appears in the rendered line.
        assert!(row.spans.iter().any(|s| s.content.contains("My Session")));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn load_session_entries_returns_empty_on_error() {
        // Given a test Services (with fake session store that returns empty).
        let services = crate::common::services::Services::new();

        // When loading session entries.
        let entries = load_session_entries(&services, &default_theme()).await;

        // Then an empty list is returned (fake store has no sessions).
        assert!(entries.is_empty());
    }
}
