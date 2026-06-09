//! Viewport computation - scroll math and visible entry determination.

/// Accumulated scroll computation results.
pub(crate) struct ScrollState {
    pub blank_count: usize,
    pub max_offset: u16,
    pub clamped: u16,
}

/// Compute scroll offset, blank count, max offset, and scroll-to-selected adjustment.
#[expect(
    clippy::else_if_without_else,
    reason = "no-op on fallthrough is intentional"
)]
pub(crate) fn compute_scroll(
    area_height: u16,
    total_wrapped: u16,
    selected_idx: Option<usize>,
    entry_line_ranges: &[(u16, u16)],
    scroll_offset: Option<u16>,
) -> ScrollState {
    let blank_count = area_height.saturating_sub(total_wrapped) as usize;
    let total_display = total_wrapped + blank_count as u16;
    let max_offset = total_display.saturating_sub(area_height);

    let resolved = scroll_offset.unwrap_or(max_offset);
    let mut clamped = resolved.min(max_offset);

    // Scroll-to-selected: adjust clamped offset to keep selected entry visible.
    if let Some(sel_idx) = selected_idx
        && let Some(&(start, end)) = entry_line_ranges.get(sel_idx)
    {
        let abs_start = start + blank_count as u16;
        let abs_end = end + blank_count as u16;
        let entry_height = abs_end.saturating_sub(abs_start);
        let viewport_top = clamped;
        let viewport_bottom = clamped.saturating_add(area_height);

        if entry_height <= area_height {
            if abs_start < viewport_top {
                clamped = abs_start;
            } else if abs_end > viewport_bottom {
                clamped = abs_end.saturating_sub(area_height);
            }
        } else if abs_start >= viewport_bottom {
            clamped = abs_start;
        } else if abs_end <= viewport_top {
            clamped = abs_end.saturating_sub(area_height);
        }
    }

    ScrollState {
        blank_count,
        max_offset,
        clamped,
    }
}

/// Determine which entry indices overlap the current viewport.
pub(crate) fn find_visible_indices(
    entry_line_ranges: &[(u16, u16)],
    blank_count: usize,
    clamped: u16,
    area_height: u16,
) -> Vec<usize> {
    let viewport_top = clamped;
    let viewport_bottom = clamped.saturating_add(area_height);

    entry_line_ranges
        .iter()
        .enumerate()
        .filter_map(|(i, &(start, end))| {
            let abs_start = start + blank_count as u16;
            let abs_end = end + blank_count as u16;
            (abs_end > viewport_top && abs_start < viewport_bottom).then_some(i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    // --- compute_scroll: scroll-to-selected logic ---

    #[rstest::rstest]
    fn compute_scroll_clamps_up_when_selected_above_viewport() {
        // Given 20 entries, each 1 line, selected entry at line 2, viewport scrolled to line 5.
        let ranges: Vec<(u16, u16)> = (0..20).map(|i| (i, i + 1)).collect();

        // When computing scroll with entry 2 selected but viewport starting at 5.
        // area_height=10, total_wrapped=20, max_offset=10, resolved=5, clamped=5.
        // viewport_top=5, viewport_bottom=15.
        // abs_start=2 < viewport_top=5 → true, so clamped = 2.
        let result = compute_scroll(10, 20, Some(2), &ranges, Some(5));

        // Then clamped scrolls up to show entry 2.
        assert_eq!(result.clamped, 2, "should scroll up to show selected entry");
    }

    #[rstest::rstest]
    fn compute_scroll_clamps_down_when_selected_below_viewport() {
        // Given 20 entries, each 1 line, selected entry at line 15, viewport at line 0.
        let ranges: Vec<(u16, u16)> = (0..20).map(|i| (i, i + 1)).collect();

        // When computing scroll with entry 15 selected but viewport starting at 0.
        let result = compute_scroll(10, 20, Some(15), &ranges, Some(0));

        // Then clamped scrolls down so entry 15's end is at viewport bottom.
        // abs_end = 16, area_height = 10, so clamped = 16 - 10 = 6.
        assert_eq!(
            result.clamped, 6,
            "should scroll down to show selected entry"
        );
    }

    #[rstest::rstest]
    fn compute_scroll_no_scroll_when_selected_already_visible() {
        // Given 5 entries, each 1 line, selected entry at line 2, viewport at line 0.
        let ranges: Vec<(u16, u16)> = (0..5).map(|i| (i, i + 1)).collect();

        // When computing scroll with entry 2 selected and viewport starting at 0.
        let result = compute_scroll(5, 5, Some(2), &ranges, Some(0));

        // Then no scroll adjustment needed.
        assert_eq!(
            result.clamped, 0,
            "no scroll when selected is already visible"
        );
    }

    #[rstest::rstest]
    fn compute_scroll_large_entry_does_not_scroll_if_partially_visible() {
        // Given an entry that spans lines 3-8 (height 5) in a viewport of height 4.
        // entry_height (5) > area_height (4), so the entry is taller than the viewport.
        let ranges: Vec<(u16, u16)> = vec![(0, 1), (1, 2), (2, 3), (3, 8)];

        // Entry 3 spans lines 3-8, viewport at 0 (viewport_bottom = 4).
        // entry_height = 5 > area_height = 4, so we enter the else branch.
        // abs_start (3) < viewport_bottom (4), so neither else-if fires.
        let result = compute_scroll(4, 8, Some(3), &ranges, Some(0));

        // entering the first branch and scrolling to abs_start = 3.
        // The correct behavior is NOT scrolling because the entry is partially visible.
        assert_eq!(
            result.clamped, 0,
            "large entry partially visible should not scroll"
        );
    }

    #[rstest::rstest]
    fn compute_scroll_large_entry_completely_below_viewport() {
        // Given an entry spanning lines 10-15 (height 5) in a viewport of height 4 starting at 0.
        let ranges: Vec<(u16, u16)> = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 15)];

        // Entry 4 spans lines 4-15, entry_height = 11 > area_height = 4.
        // viewport at 0, viewport_bottom = 4.
        // abs_start = 4 >= viewport_bottom = 4, so clamped = abs_start = 4.
        let result = compute_scroll(4, 15, Some(4), &ranges, Some(0));

        assert_eq!(
            result.clamped, 4,
            "large entry completely below should scroll to its start"
        );
    }

    #[rstest::rstest]
    fn compute_scroll_large_entry_completely_above_viewport() {
        // Given an entry spanning lines 0-5 (height 5) in a viewport of height 4 starting at 10.
        let ranges: Vec<(u16, u16)> = vec![(0, 5), (5, 6), (6, 7), (7, 8), (8, 9)];

        // Entry 0 spans lines 0-5, entry_height = 5 > area_height = 4.
        // viewport at 10, viewport_top = 10.
        // abs_end = 5 <= viewport_top = 10, so clamped = 5 - 4 = 1.
        let result = compute_scroll(4, 9, Some(0), &ranges, Some(10));

        // max_offset = 9 - 4 = 5, clamped starts at min(10, 5) = 5.
        // Wait, resolved = scroll_offset.unwrap_or(max_offset) = 10, clamped = min(10, 5) = 5.
        // Then scroll-to-selected: abs_start=0, abs_end=5, viewport_top=5, viewport_bottom=9.
        // entry_height = 5 > area_height = 4.
        // abs_start (0) < viewport_bottom (9) → false for first else-if.
        // abs_end (5) <= viewport_top (5) → true, so clamped = 5 - 4 = 1.
        assert_eq!(
            result.clamped, 1,
            "large entry completely above should scroll up"
        );
    }

    #[rstest::rstest]
    fn compute_scroll_no_selected_entry_uses_max_offset() {
        // Given no selected entry.
        let ranges: Vec<(u16, u16)> = vec![(0, 1), (1, 2), (2, 3)];

        // When computing scroll with no selected entry and no scroll_offset.
        let result = compute_scroll(5, 3, None, &ranges, None);

        // Then clamped = max_offset = 0 (content shorter than viewport).
        assert_eq!(result.clamped, 0);
        assert_eq!(result.blank_count, 2);
    }

    // --- find_visible_indices: boundary conditions ---

    #[rstest::rstest]
    fn find_visible_entries_at_viewport_boundary() {
        // Given entries at lines [0,1), [1,2), [2,3), [3,4), [4,5).
        let ranges: Vec<(u16, u16)> = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)];

        // When viewport is [1, 4).
        let visible = find_visible_indices(&ranges, 0, 1, 3);

        // Then entries that overlap [1, 4) are visible.
        // Entry 0: [0,1) -> abs_end=1 > viewport_top=1? NO (1 > 1 is false).
        // Entry 1: [1,2) -> visible
        // Entry 2: [2,3) -> visible
        // Entry 3: [3,4) -> abs_start=3 < viewport_bottom=4? YES. visible
        // Entry 4: [4,5) -> abs_start=4 < viewport_bottom=4? NO (4 < 4 is false).
        assert_eq!(visible, vec![1, 2, 3]);
    }

    #[rstest::rstest]
    fn find_visible_with_blank_count() {
        // Given entries at [0,2) and [2,5) with blank_count=3.
        let ranges: Vec<(u16, u16)> = vec![(0, 2), (2, 5)];

        // Viewport at clamped=0, area_height=10.
        // abs positions: entry 0 = [3, 5), entry 1 = [5, 8).
        let visible = find_visible_indices(&ranges, 3, 0, 10);

        // Both entries should be visible.
        assert_eq!(visible, vec![0, 1]);
    }

    #[rstest::rstest]
    fn find_visible_empty_ranges() {
        // Given no entries.
        let ranges: Vec<(u16, u16)> = vec![];

        let visible = find_visible_indices(&ranges, 0, 0, 10);

        assert!(visible.is_empty());
    }
}
