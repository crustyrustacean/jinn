//! The saved-attendants picker's viewport measurement.
//!
//! Paging needs to know how many result rows are actually on screen: the
//! selection widget keeps the highlighted row inside a visible window of
//! that size, and a wrong value lets the highlight scroll out of sight.
//! Mirrors the report-history picker's measurement exactly — same popup
//! widget, same layout arithmetic.

use ratatui::layout::Rect;

/// Input + separator rows the list pane reserves above its results, in the
/// side-by-side preview layout.
const CHROME_ROWS_LIST: u16 = 2;

/// The picker's result-row count for the given popup rectangle.
///
/// `popup` is the popup rectangle handed down by the render pass
/// ([`jinn_selection_widget::compute_popup_rect`]), not a terminal frame.
/// At least 1, so paging never windows against zero on a tiny terminal.
#[must_use]
pub fn results_viewport(popup: Rect) -> usize {
    let inner = popup.height.saturating_sub(2); // both borders

    let rows = if popup.width >= jinn_selection_widget::VERTICAL_SPLIT_MIN_WIDTH {
        // Side-by-side: the list pane spans the full content height, less
        // its own input and separator.
        inner.saturating_sub(CHROME_ROWS_LIST)
    } else {
        // Stacked: the list pane is a fixed short list, and the rest of the
        // popup is preview.
        jinn_selection_widget::HORIZONTAL_LIST_ROWS
    };
    usize::from(rows.max(1))
}
