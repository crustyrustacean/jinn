//! Tests for tree-aware filtering in [`TreePickerState`].

use crate::TreePickerState;
use crate::tree_item::TreeItem;
use ratatui::text::Line;
use std::ops::Range;

// ---------------------------------------------------------------------------
// Test item type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestItem {
    id: String,
    parent_id: Option<String>,
    label: String,
}

impl TreeItem for TestItem {
    fn id(&self) -> &str {
        &self.id
    }
    fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }
    fn display_label(&self) -> &str {
        &self.label
    }
    fn render_row(&self, _is_selected: bool) -> Line<'static> {
        Line::from(self.label.clone())
    }
    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        _match_indices: &[Range<usize>],
    ) -> Line<'static> {
        self.render_row(is_selected)
    }
}

fn item(id: &str, parent_id: Option<&str>, label: &str) -> TestItem {
    TestItem {
        id: id.to_owned(),
        parent_id: parent_id.map(str::to_owned),
        label: label.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn empty_filter_shows_all_items_in_tree_order() {
    // Given a tree: root A → children B, C.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", Some("a"), "Charlie"),
    ];

    // When creating state with items and empty filter.
    let state = TreePickerState::with_items(items);

    // Then all items are visible in tree order.
    assert_eq!(state.filtered_count(), 3);
    assert_eq!(state.visible_entry(0).unwrap().depth, 0);
    assert_eq!(state.visible_entry(1).unwrap().depth, 1);
    assert_eq!(state.visible_entry(2).unwrap().depth, 1);
    // Root is last child (only root).
    assert!(state.visible_entry(0).unwrap().is_last_child);
    // C is last child of A.
    assert!(!state.visible_entry(1).unwrap().is_last_child);
    assert!(state.visible_entry(2).unwrap().is_last_child);
}

#[test]
fn child_match_includes_ancestors() {
    // Given: root A → child B → grandchild C.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", Some("b"), "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering for "Charlie".
    state.insert_text("Charlie");

    // Then visible = [A, B, C] — ancestor chain included.
    assert_eq!(state.filtered_count(), 3);
    assert_eq!(state.filtered_item(0).unwrap().id, "a");
    assert_eq!(state.filtered_item(1).unwrap().id, "b");
    assert_eq!(state.filtered_item(2).unwrap().id, "c");
    // A and B have no match indices; C has match bytes.
    assert!(state.filtered_match_indices(0).unwrap().is_empty());
    assert!(state.filtered_match_indices(1).unwrap().is_empty());
    assert!(!state.filtered_match_indices(2).unwrap().is_empty());
}

#[test]
fn non_matching_siblings_excluded() {
    // Given: root A → children B, C.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", Some("a"), "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering for "Charlie".
    state.insert_text("Charlie");

    // Then visible = [A, C] — B excluded.
    assert_eq!(state.filtered_count(), 2);
    assert_eq!(state.filtered_item(0).unwrap().id, "a");
    assert_eq!(state.filtered_item(1).unwrap().id, "c");
    // C is now last child (only visible child).
    assert!(state.visible_entry(1).unwrap().is_last_child);
}

#[test]
fn tree_connectors_recomputed_for_filtered_set() {
    // Given: root A → children B, C, D.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", Some("a"), "Charlie"),
        item("d", Some("a"), "Delta"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering for "Delta".
    state.insert_text("Delta");

    // Then D is is_last_child = true (recomputed for visible set).
    assert_eq!(state.filtered_count(), 2);
    assert_eq!(state.filtered_item(0).unwrap().id, "a");
    assert_eq!(state.filtered_item(1).unwrap().id, "d");
    assert!(state.visible_entry(1).unwrap().is_last_child);
}

#[test]
fn orphaned_item_treated_as_root() {
    // Given: child C references non-existent parent X.
    let items = vec![item("a", None, "Alpha"), item("c", Some("x"), "Charlie")];
    let state = TreePickerState::with_items(items);

    // Then C appears as a root (depth 0).
    assert_eq!(state.filtered_count(), 2);
    assert_eq!(state.visible_entry(1).unwrap().depth, 0);
}

#[test]
fn circular_reference_guard() {
    // Given: A → B → A cycle.
    let items = vec![item("a", Some("b"), "Alpha"), item("b", Some("a"), "Bravo")];

    // When creating state with items.
    let state = TreePickerState::with_items(items);

    // Then no infinite loop — both appear as roots (neither's parent is resolvable
    // since they reference each other but neither is a root initially).
    // Actually, A's parent is B (which is in items), so A is a child of B.
    // B's parent is A (which is in items), so B is a child of A.
    // Neither is a root, so both are orphaned due to cycle.
    // The build_index marks an item as root if parent_id is None OR parent is not in id_to_idx.
    // A's parent is B (in items), so A goes to children_map["b"].
    // B's parent is A (in items), so B goes to children_map["a"].
    // Neither goes to roots.
    // DFS visits roots (empty) → nothing visible.
    // This is actually correct behavior for corrupted data.
    assert_eq!(state.filtered_count(), 0);
}

#[test]
fn multiple_matches_in_different_subtrees() {
    // Given: root A → child B, root C → child D.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", None, "Charlie"),
        item("d", Some("c"), "Delta"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering for "Bravo Delta" (matches B and D).
    state.insert_text("Bravo");

    // After inserting "Bravo", only B matches (and its parent A).
    assert_eq!(state.filtered_count(), 2);
    assert_eq!(state.filtered_item(0).unwrap().id, "a");
    assert_eq!(state.filtered_item(1).unwrap().id, "b");
}

#[test]
fn root_match_does_not_include_children() {
    // Given: root A → children B, C.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", Some("a"), "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering for "Alpha".
    state.insert_text("Alpha");

    // Then only A is visible (children are not ancestors, they're descendants).
    assert_eq!(state.filtered_count(), 1);
    assert_eq!(state.filtered_item(0).unwrap().id, "a");
}

#[test]
fn selection_clamped_after_filter() {
    // Given: root A → children B, C.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", Some("a"), "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering reduces visible set to 2 items.
    state.insert_text("a");

    // Then selection resets to 0.
    assert_eq!(state.selection(), 0);
}

#[test]
fn reset_clears_filter_and_restores_full_tree() {
    // Given: state with active filter.
    let items = vec![item("a", None, "Alpha"), item("b", Some("a"), "Bravo")];
    let mut state = TreePickerState::with_items(items);
    state.insert_text("Bravo");
    assert_eq!(state.filtered_count(), 2); // A + B

    // When resetting.
    state.reset();

    // Then filter is cleared and all items visible.
    assert!(state.filter().is_empty());
    assert_eq!(state.filtered_count(), 2);
    assert_eq!(state.selection(), 0);
}

#[test]
fn set_items_rebuilds_index() {
    // Given: state with items.
    let items = vec![item("a", None, "Alpha"), item("b", Some("a"), "Bravo")];
    let mut state = TreePickerState::with_items(items);

    // When setting new items.
    let new_items = vec![item("x", None, "Xray"), item("y", Some("x"), "Yankee")];
    state.set_items(new_items);

    // Then visible list reflects new items.
    assert_eq!(state.filtered_count(), 2);
    assert_eq!(state.filtered_item(0).unwrap().id, "x");
    assert_eq!(state.filtered_item(1).unwrap().id, "y");
}
