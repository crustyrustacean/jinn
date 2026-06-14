//! Tree-aware selection state - the core state machine for the tree picker.
//!
//! [`TreePickerState`] holds the filter text, cursor position, selection index,
//! scroll offset, the full item list, and a cached visible list with tree metadata.
//! Filter input methods trigger tree-aware fuzzy re-filtering (ancestor-chain expansion)
//! and navigation methods move the selection within the visible list.

use std::collections::{HashMap, HashSet};

use fuzzy_matcher::FuzzyMatcher as _;
use fuzzy_matcher::skim::SkimMatcherV2;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::PickerOps;
use crate::tree_item::TreeItem;

/// A visible entry in the filtered tree, with recomputed tree metadata.
#[derive(Debug, Clone)]
pub struct VisibleEntry {
    /// Index into the `items` vector.
    pub item_idx: usize,
    /// Depth in the visible tree (0 for roots).
    pub depth: usize,
    /// For each ancestor level (0..depth-1), `true` if that ancestor has younger
    /// visible siblings. Used to render `│` vs ` ` continuation characters.
    pub ancestor_continuations: Vec<bool>,
    /// Whether this entry is the last visible child of its parent.
    pub is_last_child: bool,
}

/// State machine for a tree-structured search+filter+select picker.
///
/// Generic over any type implementing [`TreeItem`]. Owns the item list and caches
/// filtered results as [`VisibleEntry`] instances with recomputed tree metadata.
///
/// # Tree-aware filtering
///
/// When the filter is empty, all items are visible in DFS tree order.
/// When a filter is active, fuzzy matching determines which items match, and
/// the ancestor chain of each matching item is included (even if the ancestors
/// don't match). Non-matching siblings are excluded. Tree metadata (depth,
/// continuations, is_last_child) is recomputed for the visible subset.
///
/// # Ordering
///
/// Items appear in tree DFS order - NOT sorted by fuzzy score. Score is only
/// used to determine which items match, not for ordering.
#[derive(Debug)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "fields need pub(crate) for cross-module test access within the crate"
)]
pub struct TreePickerState<I>
where
    I: TreeItem,
{
    /// Current filter text typed by the user.
    pub(crate) filter: String,
    /// Cursor position as a grapheme-cluster index within `filter`.
    pub(crate) cursor_pos: usize,
    /// Index of the currently highlighted item in the visible list.
    pub(crate) selection: usize,
    /// Index of the first visible result row (scroll window top).
    pub(crate) scroll_offset: usize,
    /// The full item list provided by the consumer.
    items: Vec<I>,
    /// Filtered + ancestor-expanded results with recomputed tree metadata.
    visible: Vec<VisibleEntry>,
    /// Byte-index match info per visible entry (for highlighting).
    /// Empty for ancestor-only entries (no match) and when filter is empty.
    match_indices: Vec<Vec<usize>>,
    /// Map from item ID to item index in `items`.
    id_to_idx: HashMap<String, usize>,
    /// Map from parent ID to child indices in `items`.
    children_map: HashMap<String, Vec<usize>>,
    /// Root item indices (items with no parent or orphaned).
    roots: Vec<usize>,
}

impl<I> Default for TreePickerState<I>
where
    I: TreeItem,
{
    fn default() -> Self {
        Self::new()
    }
}

#[expect(
    clippy::same_name_method,
    reason = "public API mirrors PickerOps trait methods for ergonomics"
)]
impl<I> TreePickerState<I>
where
    I: TreeItem,
{
    /// Creates a new, empty tree picker state with no items.
    #[must_use]
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            cursor_pos: 0,
            selection: 0,
            scroll_offset: 0,
            items: Vec::new(),
            visible: Vec::new(),
            match_indices: Vec::new(),
            id_to_idx: HashMap::new(),
            children_map: HashMap::new(),
            roots: Vec::new(),
        }
    }

    /// Creates a tree picker state pre-populated with items.
    ///
    /// When the filter is empty (which it is initially), all items are visible
    /// in DFS tree order.
    #[must_use]
    pub fn with_items(items: Vec<I>) -> Self {
        let mut state = Self::new();
        state.set_items(items);
        state
    }

    // --- Item management ---

    /// Replaces the full item list and re-filters against the current filter text.
    ///
    /// Does **not** reset the filter text or cursor position.
    /// Clamps `selection` to stay within the new visible bounds.
    pub fn set_items(&mut self, items: Vec<I>) {
        self.items = items;
        self.build_index();
        self.recompute_filtered();
        self.selection = self.selection.min(self.visible.len().saturating_sub(1));
    }

    // --- Filter input methods (all trigger re-filter, reset selection to 0) ---

    /// Inserts a character at the cursor position, advances the cursor, re-filters, and
    /// resets selection and scroll offset to 0.
    pub fn insert_char(&mut self, ch: char) {
        let byte_offset = self
            .filter
            .grapheme_indices(true)
            .nth(self.cursor_pos)
            .map_or(self.filter.len(), |(i, _)| i);
        self.filter.insert(byte_offset, ch);
        self.cursor_pos += 1;
        self.recompute_filtered();
        self.selection = 0;
        self.scroll_offset = 0;
    }

    /// Bulk inserts text at the cursor position, advances the cursor, re-filters, and
    /// resets selection and scroll offset to 0.
    ///
    /// Newlines are stripped - the picker filter is a single line.
    pub fn insert_text(&mut self, text: &str) {
        let flat: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        if flat.is_empty() {
            return;
        }
        let byte_offset = self
            .filter
            .grapheme_indices(true)
            .nth(self.cursor_pos)
            .map_or(self.filter.len(), |(i, _)| i);
        self.filter.insert_str(byte_offset, &flat);
        self.cursor_pos += flat.graphemes(true).count();
        self.recompute_filtered();
        self.selection = 0;
        self.scroll_offset = 0;
    }

    /// Deletes the grapheme before the cursor, decrements the cursor, re-filters, and
    /// resets selection and scroll offset to 0.
    ///
    /// No-op when the cursor is at position 0.
    pub fn backspace(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let graphemes: Vec<_> = self.filter.grapheme_indices(true).collect();
        let delete_idx = self.cursor_pos - 1;
        let Some(&(start, g)) = graphemes.get(delete_idx) else {
            return;
        };
        let end = start + g.len();
        self.filter.drain(start..end);
        self.cursor_pos -= 1;
        self.recompute_filtered();
        self.selection = 0;
        self.scroll_offset = 0;
    }

    // --- Cursor movement (do NOT trigger re-filter or reset selection) ---

    /// Moves the cursor one grapheme to the left.
    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    /// Moves the cursor one grapheme to the right.
    pub fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.filter.graphemes(true).count() {
            self.cursor_pos += 1;
        }
    }

    // --- Selection movement (do NOT trigger re-filter) ---

    /// Moves the selection up by one, clamping at 0, then adjusts scroll offset.
    pub fn move_up(&mut self, max_visible: usize) {
        if self.selection > 0 {
            self.selection -= 1;
        }
        self.ensure_visible(max_visible);
    }

    /// Moves the selection down by one, clamping at the end of the visible list,
    /// then adjusts scroll offset.
    pub fn move_down(&mut self, max_visible: usize) {
        let max = self.visible.len();
        if max > 0 && self.selection < max - 1 {
            self.selection += 1;
        }
        self.ensure_visible(max_visible);
    }

    // --- Scroll ---

    /// Adjusts `scroll_offset` so that `selection` is within the visible window.
    pub fn ensure_visible(&mut self, max_visible: usize) {
        if self.selection < self.scroll_offset {
            self.scroll_offset = self.selection;
        } else if max_visible > 0 && self.selection >= self.scroll_offset + max_visible {
            self.scroll_offset = self.selection - max_visible + 1;
        } else {
            // Selection is within the visible window - no adjustment needed.
        }
    }

    // --- Reset ---

    /// Clears the filter text and resets selection, cursor, and scroll offset to 0.
    ///
    /// Does **not** clear the item list.
    pub fn reset(&mut self) {
        self.filter.clear();
        self.selection = 0;
        self.cursor_pos = 0;
        self.scroll_offset = 0;
        self.recompute_filtered();
    }

    // --- Read access ---

    /// Returns the current filter text.
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Returns the current cursor position as a grapheme index.
    #[must_use]
    pub fn cursor_pos(&self) -> usize {
        self.cursor_pos
    }

    /// Returns the current scroll offset.
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Returns the current selection index within the visible list.
    #[must_use]
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// Returns the currently selected item, or `None` if the visible list is empty.
    #[must_use]
    pub fn selected_item(&self) -> Option<&I> {
        let entry = self.visible.get(self.selection)?;
        self.items.get(entry.item_idx)
    }

    /// Returns the number of visible entries.
    #[must_use]
    pub fn filtered_count(&self) -> usize {
        self.visible.len()
    }

    /// Returns the visible entry at the given index, or `None` if out of bounds.
    #[must_use]
    pub fn visible_entry(&self, idx: usize) -> Option<&VisibleEntry> {
        self.visible.get(idx)
    }

    /// Returns the item at the given visible index, or `None` if out of bounds.
    #[must_use]
    pub fn filtered_item(&self, idx: usize) -> Option<&I> {
        let entry = self.visible.get(idx)?;
        self.items.get(entry.item_idx)
    }

    /// Returns the full item list (all items, not just filtered).
    #[must_use]
    pub fn items(&self) -> &[I] {
        &self.items
    }

    /// Returns the fuzzy match byte indices for the visible entry at `idx`,
    /// or `None` if out of bounds.
    #[must_use]
    pub fn filtered_match_indices(&self, idx: usize) -> Option<&[usize]> {
        self.match_indices.get(idx).map(Vec::as_slice)
    }

    // --- Internal ---

    /// Builds the parent/child index from the current items.
    fn build_index(&mut self) {
        self.id_to_idx.clear();
        self.children_map.clear();
        self.roots.clear();

        // First pass: collect all IDs.
        for (idx, item) in self.items.iter().enumerate() {
            self.id_to_idx.insert(item.id().to_owned(), idx);
        }

        // Second pass: resolve parent relationships.
        for (idx, item) in self.items.iter().enumerate() {
            match item.parent_id() {
                Some(pid) if self.id_to_idx.contains_key(pid) => {
                    self.children_map
                        .entry(pid.to_owned())
                        .or_default()
                        .push(idx);
                }
                _ => {
                    self.roots.push(idx);
                }
            }
        }
    }

    /// Recomputes the visible list based on the current filter.
    fn recompute_filtered(&mut self) {
        if self.filter.is_empty() {
            // All items visible in DFS tree order.
            self.visible = Self::dfs_full(&self.roots, &self.children_map, &self.items);
            self.match_indices = vec![Vec::new(); self.visible.len()];
        } else {
            let matcher = SkimMatcherV2::default();
            let terms: Vec<&str> = self.filter.split_whitespace().collect();

            // Phase 1: Fuzzy match all items.
            let mut matched_set: HashSet<usize> = HashSet::new();
            let mut match_data: HashMap<usize, Vec<usize>> = HashMap::new();

            for (idx, item) in self.items.iter().enumerate() {
                if let Some((_score, indices)) = match_all_terms(&matcher, item, &terms) {
                    matched_set.insert(idx);
                    match_data.insert(idx, indices);
                }
            }

            // Phase 2: Collect ancestors of every matched item.
            let mut needed: HashSet<usize> = matched_set.clone();
            for &idx in &matched_set {
                Self::walk_ancestors(idx, &self.id_to_idx, &self.items, &mut needed);
            }

            // Phase 3: DFS traversal keeping only needed items.
            self.visible =
                Self::dfs_filtered(&self.roots, &self.children_map, &self.items, &needed);

            // Build match indices for visible entries.
            self.match_indices = self
                .visible
                .iter()
                .map(|entry| match_data.get(&entry.item_idx).cloned().unwrap_or_default())
                .collect();
        }
    }

    /// Walks the ancestor chain from `idx` and adds each ancestor to `needed`.
    fn walk_ancestors(
        start_idx: usize,
        id_to_idx: &HashMap<String, usize>,
        items: &[I],
        needed: &mut HashSet<usize>,
    ) {
        let mut current = start_idx;
        #[expect(clippy::while_let_loop, reason = "two independent break conditions")]
        loop {
            let Some(item) = items.get(current) else {
                break;
            };
            let Some(parent_str) = item.parent_id() else {
                break;
            };
            let Some(&parent_idx) = id_to_idx.get(parent_str) else {
                break; // orphan
            };
            if needed.contains(&parent_idx) {
                break; // already included - prevents cycles
            }
            needed.insert(parent_idx);
            current = parent_idx;
        }
    }

    /// Full DFS traversal producing visible entries with tree metadata.
    #[expect(
        clippy::indexing_slicing,
        reason = "root_idx comes from roots which are valid indices into items"
    )]
    fn dfs_full(
        roots: &[usize],
        children_map: &HashMap<String, Vec<usize>>,
        items: &[I],
    ) -> Vec<VisibleEntry> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let root_count = roots.len();

        for (i, &root_idx) in roots.iter().enumerate() {
            if !visited.insert(root_idx) {
                continue;
            }
            result.push(VisibleEntry {
                item_idx: root_idx,
                depth: 0,
                ancestor_continuations: Vec::new(),
                is_last_child: i == root_count - 1,
            });
            let parent_id = items[root_idx].id().to_owned();
            dfs_children(
                &parent_id,
                children_map,
                items,
                &mut result,
                Vec::new(),
                &mut visited,
            );
        }

        result
    }

    /// DFS traversal keeping only items in `needed`, with recomputed tree metadata.
    #[expect(
        clippy::indexing_slicing,
        reason = "root_idx comes from visible_roots which are valid indices into items"
    )]
    fn dfs_filtered(
        roots: &[usize],
        children_map: &HashMap<String, Vec<usize>>,
        items: &[I],
        needed: &HashSet<usize>,
    ) -> Vec<VisibleEntry> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();

        let visible_roots: Vec<usize> = roots
            .iter()
            .copied()
            .filter(|r| needed.contains(r))
            .collect();
        let root_count = visible_roots.len();

        for (i, &root_idx) in visible_roots.iter().enumerate() {
            if !visited.insert(root_idx) {
                continue;
            }
            result.push(VisibleEntry {
                item_idx: root_idx,
                depth: 0,
                ancestor_continuations: Vec::new(),
                is_last_child: i == root_count - 1,
            });
            let parent_id = items[root_idx].id().to_owned();
            dfs_children_filtered(
                &parent_id,
                children_map,
                items,
                needed,
                &mut result,
                Vec::new(),
                &mut visited,
            );
        }

        result
    }
}

impl<I: TreeItem> PickerOps for TreePickerState<I> {
    fn insert_char(&mut self, ch: char) {
        TreePickerState::insert_char(self, ch);
    }

    fn insert_text(&mut self, text: &str) {
        TreePickerState::insert_text(self, text);
    }

    fn backspace(&mut self) {
        TreePickerState::backspace(self);
    }

    fn move_up(&mut self, max_visible: usize) {
        TreePickerState::move_up(self, max_visible);
    }

    fn move_down(&mut self, max_visible: usize) {
        TreePickerState::move_down(self, max_visible);
    }

    fn move_cursor_left(&mut self) {
        TreePickerState::move_cursor_left(self);
    }

    fn move_cursor_right(&mut self) {
        TreePickerState::move_cursor_right(self);
    }

    fn clear_filter(&mut self) {
        TreePickerState::reset(self);
    }

    fn is_filter_empty(&self) -> bool {
        TreePickerState::filter(self).is_empty()
    }
}

// --- Free functions ---

/// Recursively emits all children in DFS order with full tree metadata.
#[expect(
    clippy::indexing_slicing,
    reason = "child_idx comes from children_map which contains valid indices into items"
)]
fn dfs_children<I: TreeItem>(
    parent_id: &str,
    children_map: &HashMap<String, Vec<usize>>,
    items: &[I],
    result: &mut Vec<VisibleEntry>,
    ancestor_continuations: Vec<bool>,
    visited: &mut HashSet<usize>,
) {
    let Some(all_children) = children_map.get(parent_id) else {
        return;
    };

    let parent_is_last = result
        .last()
        .is_some_and(|e: &VisibleEntry| e.is_last_child);
    let mut continuations = ancestor_continuations;
    continuations.push(!parent_is_last);

    for (i, &child_idx) in all_children.iter().enumerate() {
        if !visited.insert(child_idx) {
            continue;
        }
        let is_last = i == all_children.len() - 1;
        result.push(VisibleEntry {
            item_idx: child_idx,
            depth: continuations.len(),
            ancestor_continuations: continuations.clone(),
            is_last_child: is_last,
        });
        let child_id = items[child_idx].id().to_owned();
        dfs_children(
            &child_id,
            children_map,
            items,
            result,
            continuations.clone(),
            visited,
        );
    }
}

/// Recursively emits visible children (in `needed`) in DFS order, recomputing
/// tree metadata for the filtered subset.
#[expect(
    clippy::indexing_slicing,
    reason = "child_idx comes from children_map which contains valid indices into items"
)]
fn dfs_children_filtered<I: TreeItem>(
    parent_id: &str,
    children_map: &HashMap<String, Vec<usize>>,
    items: &[I],
    needed: &HashSet<usize>,
    result: &mut Vec<VisibleEntry>,
    ancestor_continuations: Vec<bool>,
    visited: &mut HashSet<usize>,
) {
    let Some(all_children) = children_map.get(parent_id) else {
        return;
    };
    let visible_children: Vec<usize> = all_children
        .iter()
        .copied()
        .filter(|c| needed.contains(c))
        .collect();
    let child_count = visible_children.len();

    let parent_is_last = result
        .last()
        .is_some_and(|e: &VisibleEntry| e.is_last_child);
    let mut continuations = ancestor_continuations;
    continuations.push(!parent_is_last);

    for (i, &child_idx) in visible_children.iter().enumerate() {
        if !visited.insert(child_idx) {
            continue;
        }
        let is_last = i == child_count - 1;
        result.push(VisibleEntry {
            item_idx: child_idx,
            depth: continuations.len(),
            ancestor_continuations: continuations.clone(),
            is_last_child: is_last,
        });
        let child_id = items[child_idx].id().to_owned();
        dfs_children_filtered(
            &child_id,
            children_map,
            items,
            needed,
            result,
            continuations.clone(),
            visited,
        );
    }
}

/// Attempts to fuzzy-match all `terms` against an item's display label.
///
/// Returns `Some((cumulative_score, unioned_byte_indices))` when every term matches,
/// or `None` if any term fails to match.
fn match_all_terms<I: TreeItem>(
    matcher: &SkimMatcherV2,
    item: &I,
    terms: &[&str],
) -> Option<(i64, Vec<usize>)> {
    let label = item.display_label();
    let mut total_score: i64 = 0;
    let mut all_byte_indices: Vec<usize> = Vec::new();

    for term in terms {
        let (score, byte_indices) = matcher.fuzzy_indices(label, term)?;
        total_score += score;
        all_byte_indices.extend_from_slice(&byte_indices);
    }

    all_byte_indices.sort_unstable();
    all_byte_indices.dedup();
    Some((total_score, all_byte_indices))
}
