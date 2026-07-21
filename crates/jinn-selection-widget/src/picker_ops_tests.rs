//! Tests for PickerOps trait dispatch.
//!
//! Verifies that all 7 trait methods work correctly when called through
//! `&mut dyn PickerOps` for both SelectionState and TreePickerState.

use crate::PickerItem;
use crate::PickerOps;
use crate::SelectionState;
use crate::TreePickerState;
use crate::tree_item::TreeItem;
use ratatui::text::Line;
use std::ops::Range;

// ---------------------------------------------------------------------------
// Test item types
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct FlatItem {
    label: String,
}

impl FlatItem {
    fn new(label: &str) -> Self {
        Self {
            label: label.to_owned(),
        }
    }
}

impl PickerItem for FlatItem {
    fn display_label(&self) -> &str {
        &self.label
    }
    fn render_row(&self, _is_selected: bool) -> Line<'static> {
        Line::from(self.label.clone())
    }
}

#[derive(Debug, Clone)]
struct TreeTestItem {
    id: String,
    parent_id: Option<String>,
    label: String,
}

impl TreeItem for TreeTestItem {
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

fn tree_item(id: &str, parent_id: Option<&str>, label: &str) -> TreeTestItem {
    TreeTestItem {
        id: id.to_owned(),
        parent_id: parent_id.map(str::to_owned),
        label: label.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// SelectionState<T> through dyn PickerOps
// ---------------------------------------------------------------------------

#[test]
fn flat_insert_char_through_trait() {
    let mut state: SelectionState<FlatItem> = SelectionState::new();
    let ops: &mut dyn PickerOps = &mut state;
    ops.insert_char('x');
    assert_eq!(state.filter(), "x");
}

#[test]
fn flat_insert_text_through_trait() {
    let mut state: SelectionState<FlatItem> = SelectionState::new();
    let ops: &mut dyn PickerOps = &mut state;
    ops.insert_text("hello");
    assert_eq!(state.filter(), "hello");
}

#[test]
fn flat_insert_text_strips_newlines_through_trait() {
    let mut state: SelectionState<FlatItem> = SelectionState::new();
    let ops: &mut dyn PickerOps = &mut state;
    ops.insert_text("a\nb\rc");
    assert_eq!(state.filter(), "abc");
}

#[test]
fn flat_backspace_through_trait() {
    let mut state: SelectionState<FlatItem> = SelectionState::new();
    state.filter = "ab".to_owned();
    state.cursor_pos = 2;
    let ops: &mut dyn PickerOps = &mut state;
    ops.backspace();
    assert_eq!(state.filter(), "a");
    assert_eq!(state.cursor_pos(), 1);
}

#[test]
fn flat_move_up_through_trait() {
    let mut state = SelectionState::with_items(vec![
        FlatItem::new("a"),
        FlatItem::new("b"),
        FlatItem::new("c"),
    ]);
    state.selection = 2;
    let ops: &mut dyn PickerOps = &mut state;
    ops.move_up(5);
    assert_eq!(state.selection(), 1);
}

#[test]
fn flat_move_down_through_trait() {
    let mut state = SelectionState::with_items(vec![
        FlatItem::new("a"),
        FlatItem::new("b"),
        FlatItem::new("c"),
    ]);
    state.selection = 0;
    let ops: &mut dyn PickerOps = &mut state;
    ops.move_down(5);
    assert_eq!(state.selection(), 1);
}

#[test]
fn flat_page_up_through_trait() {
    // Given a flat state with 20 items and selection=10.
    let items: Vec<FlatItem> = (0..20).map(|i| FlatItem::new(&i.to_string())).collect();
    let mut state = SelectionState::with_items(items);
    state.selection = 10;
    let ops: &mut dyn PickerOps = &mut state;
    ops.page_up(10);
    assert_eq!(state.selection(), 5);
}

#[test]
fn flat_page_down_through_trait() {
    // Given a flat state with 20 items and selection=0.
    let items: Vec<FlatItem> = (0..20).map(|i| FlatItem::new(&i.to_string())).collect();
    let mut state = SelectionState::with_items(items);
    state.selection = 0;
    let ops: &mut dyn PickerOps = &mut state;
    ops.page_down(10);
    assert_eq!(state.selection(), 5);
}

#[test]
fn flat_move_cursor_left_through_trait() {
    let mut state: SelectionState<FlatItem> = SelectionState::new();
    state.cursor_pos = 3;
    let ops: &mut dyn PickerOps = &mut state;
    ops.move_cursor_left();
    assert_eq!(state.cursor_pos(), 2);
}

#[test]
fn flat_move_cursor_right_through_trait() {
    let mut state: SelectionState<FlatItem> = SelectionState::new();
    state.filter = "abc".to_owned();
    state.cursor_pos = 1;
    let ops: &mut dyn PickerOps = &mut state;
    ops.move_cursor_right();
    assert_eq!(state.cursor_pos(), 2);
}

// ---------------------------------------------------------------------------
// TreePickerState<I> through dyn PickerOps
// ---------------------------------------------------------------------------

#[test]
fn tree_insert_char_through_trait() {
    let mut state = TreePickerState::with_items(vec![tree_item("a", None, "Alpha")]);
    let ops: &mut dyn PickerOps = &mut state;
    ops.insert_char('x');
    assert_eq!(state.filter(), "x");
}

#[test]
fn tree_insert_text_through_trait() {
    let mut state = TreePickerState::with_items(vec![tree_item("a", None, "Alpha")]);
    let ops: &mut dyn PickerOps = &mut state;
    ops.insert_text("hello");
    assert_eq!(state.filter(), "hello");
}

#[test]
fn tree_insert_text_strips_newlines_through_trait() {
    let mut state = TreePickerState::with_items(vec![tree_item("a", None, "Alpha")]);
    let ops: &mut dyn PickerOps = &mut state;
    ops.insert_text("a\nb\rc");
    assert_eq!(state.filter(), "abc");
}

#[test]
fn tree_backspace_through_trait() {
    let mut state = TreePickerState::with_items(vec![tree_item("a", None, "Alpha")]);
    state.filter = "ab".to_owned();
    state.cursor_pos = 2;
    let ops: &mut dyn PickerOps = &mut state;
    ops.backspace();
    assert_eq!(state.filter(), "a");
    assert_eq!(state.cursor_pos(), 1);
}

#[test]
fn tree_move_up_through_trait() {
    let items = vec![
        tree_item("a", None, "Alpha"),
        tree_item("b", Some("a"), "Bravo"),
        tree_item("c", Some("a"), "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);
    state.selection = 2;
    let ops: &mut dyn PickerOps = &mut state;
    ops.move_up(5);
    assert_eq!(state.selection(), 1);
}

#[test]
fn tree_move_down_through_trait() {
    let items = vec![
        tree_item("a", None, "Alpha"),
        tree_item("b", None, "Bravo"),
        tree_item("c", None, "Charlie"),
    ];
    let mut state = TreePickerState::with_items(items);
    state.selection = 0;
    let ops: &mut dyn PickerOps = &mut state;
    ops.move_down(5);
    assert_eq!(state.selection(), 1);
}

#[test]
fn tree_page_up_through_trait() {
    // Given a tree with 20 root items and selection=10.
    let items: Vec<TreeTestItem> = (0..20)
        .map(|i| tree_item(&i.to_string(), None, &format!("Item{i}")))
        .collect();
    let mut state = TreePickerState::with_items(items);
    state.selection = 10;
    let ops: &mut dyn PickerOps = &mut state;
    ops.page_up(10);
    assert_eq!(state.selection(), 5);
}

#[test]
fn tree_page_down_through_trait() {
    // Given a tree with 20 root items and selection=0.
    let items: Vec<TreeTestItem> = (0..20)
        .map(|i| tree_item(&i.to_string(), None, &format!("Item{i}")))
        .collect();
    let mut state = TreePickerState::with_items(items);
    state.selection = 0;
    let ops: &mut dyn PickerOps = &mut state;
    ops.page_down(10);
    assert_eq!(state.selection(), 5);
}

#[test]
fn tree_move_cursor_left_through_trait() {
    let mut state = TreePickerState::with_items(vec![tree_item("a", None, "Alpha")]);
    state.cursor_pos = 3;
    let ops: &mut dyn PickerOps = &mut state;
    ops.move_cursor_left();
    assert_eq!(state.cursor_pos(), 2);
}

#[test]
fn tree_move_cursor_right_through_trait() {
    let mut state = TreePickerState::with_items(vec![tree_item("a", None, "Alpha")]);
    state.filter = "abc".to_owned();
    state.cursor_pos = 1;
    let ops: &mut dyn PickerOps = &mut state;
    ops.move_cursor_right();
    assert_eq!(state.cursor_pos(), 2);
}
