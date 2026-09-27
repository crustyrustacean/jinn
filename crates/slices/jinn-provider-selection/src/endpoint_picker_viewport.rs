//! The endpoint picker's viewport measurement.
//!
//! Paging (`PgUp`/`PgDn`) needs to know how many result rows are actually on
//! screen: the selection widget keeps the highlighted row inside a visible
//! window of that size, and a wrong value lets the highlight scroll out of
//! sight.
//!
//! The kernel used to publish this measurement into its own state once per
//! frame, and a picker in kernel scope could read it back. A slice-owned picker
//! has no such state, so it measures in its own renderer instead — same layout
//! arithmetic, same numbers, no shared field.

use ratatui::layout::Rect;

/// Input + separator rows the list pane reserves above its results, in the
/// side-by-side preview layout.
const CHROME_ROWS_LIST: u16 = 2;

/// The endpoint picker's result-row count for the given popup rectangle.
///
///
/// `popup` is the popup rectangle handed down by the render pass (see
/// `jinn_selection_widget::compute_popup_rect`), not a terminal frame.
///
/// Mirrors the preview widget's own split decision, so the value tracks what the
/// user can see. At least 1, so paging never windows against zero on a tiny
/// terminal.
#[must_use]
pub fn results_viewport(popup: Rect) -> usize {
    let inner = popup.height.saturating_sub(2); // both borders

    if popup.width >= jinn_selection_widget::VERTICAL_SPLIT_MIN_WIDTH {
        // Side-by-side: the list pane spans the full content height, less its
        // own input and separator.
        inner.saturating_sub(CHROME_ROWS_LIST)
    } else {
        // Stacked: the list pane is a fixed short list, and the rest of the
        // popup is preview.
        jinn_selection_widget::HORIZONTAL_LIST_ROWS
    }
    .max(1) as usize
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use super::*;

    #[rstest::rstest]
    fn wide_frame_measures_the_side_by_side_list_pane() {
        // Given a frame wide enough for the list and preview to sit side by side.
        let frame = Rect::new(0, 0, 200, 50);

        // When measuring the picker's results.
        let popup = jinn_selection_widget::compute_popup_rect(frame);
        let rows = results_viewport(popup);

        // Then the rows come from the full popup height less the borders and
        // the list pane's own input and separator.
        let expected = popup
            .height
            .saturating_sub(2)
            .saturating_sub(CHROME_ROWS_LIST);
        assert_eq!(rows, expected as usize);
    }

    #[rstest::rstest]
    fn narrow_frame_measures_the_fixed_stacked_list() {
        // Given a frame too narrow to split the list and preview.
        let frame = Rect::new(0, 0, 70, 50);

        // When measuring the picker's results.
        let popup = jinn_selection_widget::compute_popup_rect(frame);
        let rows = results_viewport(popup);

        // Then the stacked list's fixed row count applies, not the tall layout.
        assert_eq!(rows, jinn_selection_widget::HORIZONTAL_LIST_ROWS as usize);
    }

    #[rstest::rstest]
    fn a_tiny_frame_still_reports_at_least_one_row() {
        // Given a terminal too small to fit the popup's chrome.
        let frame = Rect::new(0, 0, 8, 3);

        // When measuring the picker's results.
        let popup = jinn_selection_widget::compute_popup_rect(frame);
        let rows = results_viewport(popup);

        // Then paging has a non-zero window and cannot underflow.
        assert!(rows >= 1, "viewport must never be zero, got {rows}");
    }
}
