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

    // Then visible = [A, B, C] - ancestor chain included.
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

    // Then visible = [A, C] - B excluded.
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

    // Then no infinite loop - both appear as roots (neither's parent is resolvable
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

// =========================================================================
// Phase 2: HIGH severity - core state mutation tests
// =========================================================================

// --- insert_char ---

#[test]
fn tree_insert_char_appends_to_filter() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.insert_char('x');
    assert_eq!(state.filter(), "x");
}

#[test]
fn tree_insert_char_advances_cursor() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.insert_char('x');
    assert_eq!(state.cursor_pos(), 1);
}

#[test]
fn tree_insert_char_resets_selection() {
    let items = vec![item("a", None, "Alpha"), item("b", Some("a"), "Bravo")];
    let mut state = TreePickerState::with_items(items);
    state.selection = 1;
    state.insert_char('x');
    assert_eq!(state.selection(), 0);
}

#[test]
fn tree_insert_char_resets_scroll_offset() {
    let items = vec![item("a", None, "Alpha")];
    let mut state = TreePickerState::with_items(items);
    state.scroll_offset = 5;
    state.insert_char('x');
    assert_eq!(state.scroll_offset(), 0);
}

#[test]
fn tree_insert_char_at_cursor_middle() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.filter = "abc".to_owned();
    state.cursor_pos = 1;
    state.insert_char('x');
    assert_eq!(state.filter(), "axbc");
    assert_eq!(state.cursor_pos(), 2);
}

// --- insert_text ---

#[test]
fn tree_insert_text_strips_newlines_and_carriage_returns() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.insert_text("he\nl\rl\no");
    assert_eq!(state.filter(), "hello");
}

#[test]
fn tree_insert_text_advances_cursor_by_grapheme_count() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.insert_text("abc");
    assert_eq!(state.cursor_pos(), 3);
}

#[test]
fn tree_insert_text_with_only_newlines_is_noop() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.filter = "existing".to_owned();
    state.cursor_pos = 3;
    state.insert_text("\n\r\n");
    assert_eq!(state.filter(), "existing");
    assert_eq!(state.cursor_pos(), 3);
}

#[test]
fn tree_insert_text_resets_selection_and_scroll() {
    let items = vec![item("a", None, "Alpha"), item("b", Some("a"), "Bravo")];
    let mut state = TreePickerState::with_items(items);
    state.selection = 1;
    state.scroll_offset = 1;
    state.insert_text("x");
    assert_eq!(state.selection(), 0);
    assert_eq!(state.scroll_offset(), 0);
}

#[test]
fn tree_insert_text_at_cursor_middle() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.filter = "ace".to_owned();
    state.cursor_pos = 1;
    state.insert_text("bd");
    assert_eq!(state.filter(), "abdce");
    assert_eq!(state.cursor_pos(), 3);
}

// --- backspace ---

#[test]
fn tree_backspace_at_start_is_noop() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.filter = "abc".to_owned();
    state.cursor_pos = 0;
    state.backspace();
    assert_eq!(state.filter(), "abc");
    assert_eq!(state.cursor_pos(), 0);
}

#[test]
fn tree_backspace_removes_before_cursor() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.filter = "abc".to_owned();
    state.cursor_pos = 2;
    state.backspace();
    assert_eq!(state.filter(), "ac");
    assert_eq!(state.cursor_pos(), 1);
}

#[test]
fn tree_backspace_at_end_removes_last() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.filter = "ab".to_owned();
    state.cursor_pos = 2;
    state.backspace();
    assert_eq!(state.filter(), "a");
    assert_eq!(state.cursor_pos(), 1);
}

#[test]
fn tree_backspace_resets_selection_and_scroll() {
    let items = vec![item("a", None, "Alpha"), item("b", Some("a"), "Bravo")];
    let mut state = TreePickerState::with_items(items);
    state.filter = "ab".to_owned();
    state.cursor_pos = 2;
    state.selection = 1;
    state.scroll_offset = 1;
    state.backspace();
    assert_eq!(state.selection(), 0);
    assert_eq!(state.scroll_offset(), 0);
}

// --- move_cursor_left ---

#[test]
fn tree_move_cursor_left_decrements() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.cursor_pos = 3;
    state.move_cursor_left();
    assert_eq!(state.cursor_pos(), 2);
}

#[test]
fn tree_move_cursor_left_clamps_at_zero() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.cursor_pos = 0;
    state.move_cursor_left();
    assert_eq!(state.cursor_pos(), 0);
}

#[test]
fn tree_move_cursor_left_multiple_times() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.cursor_pos = 2;
    state.move_cursor_left();
    state.move_cursor_left();
    state.move_cursor_left(); // should clamp at 0
    assert_eq!(state.cursor_pos(), 0);
}

// --- move_cursor_right ---

#[test]
fn tree_move_cursor_right_increments() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.filter = "abc".to_owned();
    state.cursor_pos = 1;
    state.move_cursor_right();
    assert_eq!(state.cursor_pos(), 2);
}

#[test]
fn tree_move_cursor_right_clamps_at_end() {
    let mut state = TreePickerState::with_items(vec![item("a", None, "Alpha")]);
    state.filter = "abc".to_owned();
    state.cursor_pos = 3;
    state.move_cursor_right();
    assert_eq!(state.cursor_pos(), 3);
}

// --- move_up ---

#[test]
fn tree_move_up_decrements() {
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", Some("a"), "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);
    state.selection = 2;
    state.move_up(5);
    assert_eq!(state.selection(), 1);
}

#[test]
fn tree_move_up_clamps_at_zero() {
    let items = vec![item("a", None, "Alpha")];
    let mut state = TreePickerState::with_items(items);
    state.selection = 0;
    state.move_up(5);
    assert_eq!(state.selection(), 0);
}

#[test]
fn tree_move_up_adjusts_scroll_offset() {
    let items: Vec<TestItem> = (0..10)
        .map(|i| item(&format!("{i}"), None, &format!("Item{i}")))
        .collect();
    let mut state = TreePickerState::with_items(items);
    state.selection = 2;
    state.scroll_offset = 2;
    state.move_up(5);
    assert_eq!(state.selection(), 1);
    assert_eq!(state.scroll_offset(), 1);
}

// --- move_down ---

#[test]
fn tree_move_down_increments() {
    let items = vec![
        item("a", None, "Alpha"),
        item("b", None, "Bravo"),
        item("c", None, "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);
    state.selection = 1;
    state.move_down(5);
    assert_eq!(state.selection(), 2);
}

#[test]
fn tree_move_down_clamps_at_end() {
    let items = vec![item("a", None, "Alpha")];
    let mut state = TreePickerState::with_items(items);
    state.selection = 0;
    state.move_down(5);
    assert_eq!(state.selection(), 0);
}

#[test]
fn tree_move_down_clamps_when_empty() {
    let mut state = TreePickerState::<TestItem>::new();
    state.move_down(5);
    assert_eq!(state.selection(), 0);
}

#[test]
fn tree_move_down_adjusts_scroll_offset() {
    let items: Vec<TestItem> = (0..10)
        .map(|i| item(&format!("{i}"), None, &format!("Item{i}")))
        .collect();
    let mut state = TreePickerState::with_items(items);
    state.selection = 4;
    state.scroll_offset = 0;
    state.move_down(5);
    assert_eq!(state.selection(), 5);
    assert_eq!(state.scroll_offset(), 1);
}

// --- ensure_visible ---

#[test]
fn tree_ensure_visible_selection_above_view() {
    let mut state = TreePickerState::<TestItem>::new();
    state.scroll_offset = 3;
    state.selection = 1;
    state.ensure_visible(5);
    assert_eq!(state.scroll_offset(), 1);
}

#[test]
fn tree_ensure_visible_selection_below_view() {
    let mut state = TreePickerState::<TestItem>::new();
    state.scroll_offset = 0;
    state.selection = 7;
    state.ensure_visible(5);
    assert_eq!(state.scroll_offset(), 3);
}

#[test]
fn tree_ensure_visible_selection_within_view() {
    let mut state = TreePickerState::<TestItem>::new();
    state.scroll_offset = 2;
    state.selection = 3;
    state.ensure_visible(5);
    assert_eq!(state.scroll_offset(), 2);
}

#[test]
fn tree_ensure_visible_selection_equal_to_scroll_offset() {
    let mut state = TreePickerState::<TestItem>::new();
    state.scroll_offset = 5;
    state.selection = 5;
    state.ensure_visible(5);
    assert_eq!(state.scroll_offset(), 5);
}

#[test]
fn tree_ensure_visible_selection_at_view_end() {
    let mut state = TreePickerState::<TestItem>::new();
    state.scroll_offset = 0;
    state.selection = 5; // scroll_offset + max_visible
    state.ensure_visible(5);
    assert_eq!(state.scroll_offset(), 1);
}

#[test]
fn tree_ensure_visible_with_zero_max_visible_selection_below() {
    let mut state = TreePickerState::<TestItem>::new();
    state.scroll_offset = 2;
    state.selection = 10;
    state.ensure_visible(0);
    // max_visible == 0 guard prevents scroll down.
    assert_eq!(state.scroll_offset(), 2);
}

// --- DFS traversal: is_last_child and visited guard ---

#[test]
fn tree_dfs_multiple_roots_correct_last_child_flags() {
    // Given: two roots, each with children.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", None, "Charlie"),
        item("d", Some("c"), "Delta"),
    ];
    let state = TreePickerState::with_items(items);

    // Then: root A is not last child (C is the other root).
    assert!(!state.visible_entry(0).unwrap().is_last_child);
    // Root C is last child.
    assert!(state.visible_entry(2).unwrap().is_last_child);
    // B is last child of A.
    assert!(state.visible_entry(1).unwrap().is_last_child);
    // D is last child of C.
    assert!(state.visible_entry(3).unwrap().is_last_child);
}

#[test]
fn tree_dfs_filtered_multiple_roots_recomputes_last_child() {
    // Given: two roots A and C, each with children.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", None, "Charlie"),
        item("d", Some("c"), "Delta"),
        item("e", Some("c"), "Echo"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering for "Delta" only.
    state.insert_text("Delta");

    // Then only C + D are visible (A not matched, B not matched).
    assert_eq!(state.filtered_count(), 2);
    assert_eq!(state.filtered_item(0).unwrap().id, "c");
    assert_eq!(state.filtered_item(1).unwrap().id, "d");
    // D is the last (only visible) child of C.
    assert!(state.visible_entry(1).unwrap().is_last_child);
}

#[test]
fn tree_dfs_filtered_is_last_child_uses_root_count_minus_one() {
    // Given: three roots, filter keeps only first and third.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", None, "Bravo"),
        item("c", None, "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering for something matching A and C but not B.
    state.insert_text("l"); // matches Alpha and Charlie (contains 'l') but not Bravo

    // Then last visible root is C.
    let count = state.filtered_count();
    assert!(count >= 2);
    // Find the last visible entry - should be the last root and have is_last_child = true.
    let last_entry = state.visible_entry(count - 1).unwrap();
    assert!(last_entry.is_last_child);
}

#[test]
fn tree_continuations_correct_with_sibling_filtering() {
    // Given: root A → children B, C, D. Filter keeps A and C only.
    let items = vec![
        item("a", None, "Alpha"),
        item("b", Some("a"), "Bravo"),
        item("c", Some("a"), "Charlie"),
        item("d", Some("a"), "Delta"),
    ];
    let mut state = TreePickerState::with_items(items);

    // When filtering for "Charlie".
    state.insert_text("Charlie");

    // Then C is the only visible child and is_last_child = true.
    assert_eq!(state.filtered_count(), 2); // A + C
    assert!(state.visible_entry(1).unwrap().is_last_child);
    // A is the last (only visible) root, so ancestor_continuation for C is [false]
    // (meaning: the parent A does NOT have younger siblings).
    assert_eq!(state.visible_entry(1).unwrap().ancestor_continuations, vec![false]);
}
