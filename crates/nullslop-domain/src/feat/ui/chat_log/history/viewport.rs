//! Viewport computation — scroll math and visible entry determination.

/// Accumulated scroll computation results.
pub(crate) struct ScrollState {
    pub blank_count: usize,
    pub max_offset: u16,
    pub clamped: u16,
}

/// Compute scroll offset, blank count, max offset, and scroll-to-selected adjustment.
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
            if abs_end > viewport_top && abs_start < viewport_bottom {
                Some(i)
            } else {
                None
            }
        })
        .collect()
}
