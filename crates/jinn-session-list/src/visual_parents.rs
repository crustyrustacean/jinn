//! Pure visual-parent maintenance for visible session trees.

use std::collections::{HashMap, HashSet};

use jinn_core_types::SessionId;

/// Repairs visual-parent overrides before a session is removed from the loaded set.
///
/// `removed_parent` is the removed session's persisted direct parent. Direct
/// parent ids that are still loaded take precedence; otherwise the existing
/// visual override for that parent or for the removed session is used. Both
/// direct children and transitive overrides pointing at the removed session are
/// rewritten to the resolved ancestor, or removed when no ancestor remains.
pub fn repair_visual_parents_on_removal<I>(
    visual_parents: &mut HashMap<SessionId, SessionId>,
    removed_id: &SessionId,
    removed_parent: Option<&SessionId>,
    loaded_ids: &HashSet<SessionId>,
    direct_child_ids: I,
) where
    I: IntoIterator<Item = SessionId>,
{
    let effective_ancestor =
        resolve_effective_ancestor(visual_parents, removed_id, removed_parent, loaded_ids);
    rewrite_mapping(
        visual_parents,
        direct_child_ids,
        effective_ancestor.as_ref(),
    );
    rewrite_transitive_overrides(visual_parents, removed_id, effective_ancestor.as_ref());
}

fn resolve_effective_ancestor(
    visual_parents: &HashMap<SessionId, SessionId>,
    removed_id: &SessionId,
    removed_parent: Option<&SessionId>,
    loaded_ids: &HashSet<SessionId>,
) -> Option<SessionId> {
    match removed_parent {
        Some(parent_id) if loaded_ids.contains(parent_id) => Some(parent_id.clone()),
        Some(parent_id) => visual_parents
            .get(parent_id)
            .or_else(|| visual_parents.get(removed_id))
            .cloned(),
        None => visual_parents.get(removed_id).cloned(),
    }
}

fn rewrite_transitive_overrides(
    visual_parents: &mut HashMap<SessionId, SessionId>,
    removed_id: &SessionId,
    effective_ancestor: Option<&SessionId>,
) {
    let keys = visual_parents
        .iter()
        .filter(|(_, parent)| *parent == removed_id)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    rewrite_mapping(visual_parents, keys, effective_ancestor);
}

fn rewrite_mapping<I>(
    visual_parents: &mut HashMap<SessionId, SessionId>,
    ids: I,
    effective_ancestor: Option<&SessionId>,
) where
    I: IntoIterator<Item = SessionId>,
{
    for id in ids {
        match effective_ancestor {
            Some(ancestor) => {
                visual_parents.insert(id, ancestor.clone());
            }
            None => {
                visual_parents.remove(&id);
            }
        }
    }
}

/// Removes visual-parent overrides that bypass a session loaded back into the
/// visible set. Overrides keyed by the loaded session are preserved because the
/// loaded session itself may still have a hidden parent.
pub fn clear_visual_parents_on_load(
    visual_parents: &mut HashMap<SessionId, SessionId>,
    loaded_id: &SessionId,
) {
    visual_parents.retain(|_session_id, visual_parent| visual_parent != loaded_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn removal_reparents_direct_and_transitive_children_to_loaded_grandparent() {
        // Given a removed child, its loaded parent, and two dependent rows.
        let root = SessionId::new();
        let removed = SessionId::new();
        let child = SessionId::new();
        let transitive = SessionId::new();
        let mut visual_parents = HashMap::from([(transitive.clone(), removed.clone())]);

        // When repairing before removing the middle session.
        repair_visual_parents_on_removal(
            &mut visual_parents,
            &removed,
            Some(&root),
            &HashSet::from([root.clone()]),
            [child.clone()],
        );

        // Then direct and transitive rows point to the loaded root.
        assert_eq!(visual_parents.get(&child), Some(&root));
        assert_eq!(visual_parents.get(&transitive), Some(&root));
    }

    #[rstest::rstest]
    fn removal_without_ancestor_removes_visual_overrides() {
        // Given a root removal with only visual-dependent rows.
        let removed = SessionId::new();
        let child = SessionId::new();
        let transitive = SessionId::new();
        let mut visual_parents = HashMap::from([
            (child.clone(), removed.clone()),
            (transitive.clone(), removed.clone()),
        ]);

        // When repairing before removal.
        repair_visual_parents_on_removal(
            &mut visual_parents,
            &removed,
            None,
            &HashSet::new(),
            [child],
        );

        // Then no override bypasses the removed root.
        assert!(visual_parents.is_empty());
    }

    #[rstest::rstest]
    fn loading_a_session_clears_values_but_preserves_keys() {
        // Given one override keyed by the loaded session and one pointing to it.
        let loaded = SessionId::new();
        let hidden = SessionId::new();
        let visual_parents =
            &mut HashMap::from([(loaded.clone(), hidden), (SessionId::new(), loaded.clone())]);

        // When invalidating bypasses after load.
        clear_visual_parents_on_load(visual_parents, &loaded);

        // Then only the loaded session's own key is preserved.
        assert_eq!(visual_parents.len(), 1);
        assert!(visual_parents.contains_key(&loaded));
    }
}
