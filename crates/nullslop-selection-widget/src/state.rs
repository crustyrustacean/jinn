//! Selection state — the core state machine for the picker widget.
//!
//! [`SelectionState`] holds the filter text, cursor position, selection index, scroll offset,
//! the full item list, and a cached filtered index list. Filter input methods trigger
//! fuzzy re-filtering and reset the selection. Navigation methods move the selection
//! within the filtered results and adjust the scroll window.

use fuzzy_matcher::FuzzyMatcher as _;
use fuzzy_matcher::skim::SkimMatcherV2;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::PickerItem;

/// State machine for a search+filter+select picker.
///
/// Generic over any type implementing [`PickerItem`]. Owns the item list and caches
/// filtered results as **indices** into that list (no cloning on every keystroke).
///
/// # Examples
///
/// ```ignore
/// let mut state = SelectionState::new();
/// state.set_items(vec![MyItem::new("hello"), MyItem::new("world")]);
/// state.insert_char('h');
/// assert_eq!(state.filtered_count(), 1);
/// ```
#[derive(Debug)]
pub struct SelectionState<T>
where
    T: PickerItem,
{
    /// Current filter text typed by the user.
    filter: String,
    /// Cursor position as a grapheme-cluster index within `filter` (0 = before first grapheme).
    cursor_pos: usize,
    /// Index of the currently highlighted item in the filtered list.
    selection: usize,
    /// Index of the first visible result row (scroll window top).
    scroll_offset: usize,
    /// The full item list provided by the consumer (pre-sorted).
    items: Vec<T>,
    /// Cached indices into `items` for matching entries, recomputed on filter change.
    filtered_indices: Vec<usize>,
    /// Cached byte-index vectors from fuzzy matching, one per filtered entry.
    /// Empty when filter is empty (no highlighting). Recomputed alongside `filtered_indices`.
    filtered_match_indices: Vec<Vec<usize>>,
}

impl<T> Default for SelectionState<T>
where
    T: PickerItem,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SelectionState<T>
where
    T: PickerItem,
{
    /// Creates a new, empty selection state with no items.
    #[must_use]
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            cursor_pos: 0,
            selection: 0,
            scroll_offset: 0,
            items: Vec::new(),
            filtered_indices: Vec::new(),
            filtered_match_indices: Vec::new(),
        }
    }

    /// Creates a selection state pre-populated with items.
    ///
    /// When the filter is empty (which it is initially), all items are visible.
    #[must_use]
    pub fn with_items(items: Vec<T>) -> Self {
        let filtered_indices = (0..items.len()).collect();
        let filtered_match_indices = vec![Vec::new(); items.len()];
        Self {
            filter: String::new(),
            cursor_pos: 0,
            selection: 0,
            scroll_offset: 0,
            items,
            filtered_indices,
            filtered_match_indices,
        }
    }

    // --- Item management ---

    /// Replaces the full item list and re-filters against the current filter text.
    ///
    /// Does **not** reset the filter text or cursor position — the consumer may want to
    /// update items while the picker is open (e.g., after a model cache refresh).
    /// Clamps `selection` to stay within the new filtered bounds.
    pub fn set_items(&mut self, items: Vec<T>) {
        self.items = items;
        self.recompute_filtered();
        self.selection = self
            .selection
            .min(self.filtered_indices.len().saturating_sub(1));
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
    ///
    /// No-op when the cursor is already at position 0.
    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    /// Moves the cursor one grapheme to the right.
    ///
    /// No-op when the cursor is already at the end of the filter text.
    pub fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.filter.graphemes(true).count() {
            self.cursor_pos += 1;
        }
    }

    // --- Selection movement (do NOT trigger re-filter) ---

    /// Moves the selection up by one, clamping at 0, then adjusts scroll offset.
    ///
    /// `max_visible` is the number of visible rows in the picker area, used to keep
    /// the selection within the scroll window.
    pub fn move_up(&mut self, max_visible: usize) {
        if self.selection > 0 {
            self.selection -= 1;
        }
        self.ensure_visible(max_visible);
    }

    /// Moves the selection down by one, clamping at the end of the filtered list,
    /// then adjusts scroll offset.
    ///
    /// `max_visible` is the number of visible rows in the picker area, used to keep
    /// the selection within the scroll window.
    pub fn move_down(&mut self, max_visible: usize) {
        let max = self.filtered_indices.len();
        if max > 0 && self.selection < max - 1 {
            self.selection += 1;
        }
        self.ensure_visible(max_visible);
    }

    // --- Scroll ---

    /// Adjusts `scroll_offset` so that `selection` is within the visible window.
    ///
    /// `max_visible` is the number of rows that fit in the picker area.
    pub fn ensure_visible(&mut self, max_visible: usize) {
        if self.selection < self.scroll_offset {
            self.scroll_offset = self.selection;
        } else if max_visible > 0 && self.selection >= self.scroll_offset + max_visible {
            self.scroll_offset = self.selection - max_visible + 1;
        } else {
            // Selection is within the visible window — no adjustment needed.
        }
    }

    // --- Reset ---

    /// Clears the filter text and resets selection, cursor, and scroll offset to 0.
    ///
    /// Does **not** clear the item list — the consumer manages the item lifecycle.
    /// The filtered list is recomputed (all items are visible when filter is empty).
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

    /// Returns the current scroll offset (index of first visible result row).
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Returns the current selection index within the filtered list.
    #[must_use]
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// Sets the selection index directly.
    ///
    /// Clamps to the valid range `[0, filtered_count - 1]`.
    /// Primarily for test setup — production code should use [`move_up`](Self::move_up)
    /// and [`move_down`](Self::move_down).
    pub fn set_selection(&mut self, idx: usize) {
        let max = self.filtered_indices.len();
        self.selection = if max > 0 { idx.min(max - 1) } else { 0 };
    }

    /// Returns the currently selected item, or `None` if the filtered list is empty.
    #[must_use]
    pub fn selected_item(&self) -> Option<&T> {
        let &i = self.filtered_indices.get(self.selection)?;
        self.items.get(i)
    }

    /// Returns the number of items in the filtered list.
    #[must_use]
    pub fn filtered_count(&self) -> usize {
        self.filtered_indices.len()
    }

    /// Returns the filtered item at the given index, or `None` if out of bounds.
    #[must_use]
    pub fn filtered_item(&self, idx: usize) -> Option<&T> {
        let &i = self.filtered_indices.get(idx)?;
        self.items.get(i)
    }

    /// Returns the full item list (all items, not just filtered).
    #[must_use]
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// Returns the fuzzy match byte indices for the filtered item at `idx`,
    /// or `None` if out of bounds.
    ///
    /// When the filter is empty, returns `Some(&[])` (no highlighting).
    #[must_use]
    pub fn filtered_match_indices(&self, idx: usize) -> Option<&[usize]> {
        self.filtered_match_indices.get(idx).map(Vec::as_slice)
    }

    // --- Internal ---

    /// Recomputes the filtered index cache based on the current filter text.
    ///
    /// When the filter is empty, all items are included in original order.
    /// Otherwise, the filter is split on whitespace into terms and each term must
    /// independently fuzzy-match the item's [`display_label`](PickerItem::display_label)
    /// (AND logic). Results are sorted by cumulative match score (descending), with
    /// ties broken by original item index. Match byte indices from all terms are
    /// union'd for highlighting.
    fn recompute_filtered(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
            self.filtered_match_indices = vec![Vec::new(); self.items.len()];
        } else {
            let matcher = SkimMatcherV2::default();
            let terms: Vec<&str> = self.filter.split_whitespace().collect();

            let mut scored: Vec<(i64, usize, Vec<usize>)> = Vec::new();
            for (i, item) in self.items.iter().enumerate() {
                let label = item.display_label();
                let mut total_score: i64 = 0;
                let mut all_byte_indices: Vec<usize> = Vec::new();
                let mut all_match = true;

                for term in &terms {
                    if let Some((score, byte_indices)) = matcher.fuzzy_indices(label, term) {
                        total_score += score;
                        all_byte_indices.extend_from_slice(&byte_indices);
                    } else {
                        all_match = false;
                        break;
                    }
                }

                if all_match {
                    all_byte_indices.sort_unstable();
                    all_byte_indices.dedup();
                    scored.push((total_score, i, all_byte_indices));
                }
            }

            // Sort by score descending, then by original index ascending for stable ordering.
            scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

            self.filtered_indices = scored.iter().map(|(_, i, _)| *i).collect();
            self.filtered_match_indices = scored.into_iter().map(|(_, _, idx)| idx).collect();
        }
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod state_tests;
