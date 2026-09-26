//! The project picker's viewport measurement.
//!
//! Paging (`PgUp`/`PgDn`) needs to know how many result rows are on screen: the
//! selection widget keeps the highlighted row inside a visible window of that
//! size, and a wrong value lets the highlight scroll out of sight.
//!
//! The kernel used to publish this into its own state once per frame. A
//! slice-owned picker has no such state, so it measures in its own renderer
//! instead — same layout arithmetic, same numbers, no shared field.

use jinn_project_msg::RESULTS_VIEWPORT_FALLBACK;
use ratatui::layout::Rect;

/// The project picker's result-row count for a frame of the given size.
///
/// The project picker is a single-pane list, so this is the popup's inner
/// height less its borders and the filter/separator chrome, and the keybind
/// footer. At least 1, so paging never windows against zero on a tiny
/// terminal.
#[must_use]
pub fn results_viewport(frame_area: Rect) -> usize {
    let popup = jinn_selection_widget::compute_popup_rect(frame_area);
    let inner = popup.height.saturating_sub(2); // both borders
    // Filter row + separator, and the one keybind footer row the spec draws.
    const CHROME_ROWS: u16 = 3;
    inner.saturating_sub(CHROME_ROWS).max(1) as usize
}

/// The viewport to use when no frame has been measured yet.
#[must_use]
pub fn unmeasured_viewport() -> usize {
    RESULTS_VIEWPORT_FALLBACK
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
    fn a_tall_frame_measures_the_popup_minus_its_chrome() {
        // Given a frame tall enough for the popup's max height.
        let frame = Rect::new(0, 0, 120, 50);

        // When measuring the picker's results.
        let rows = results_viewport(frame);

        // Then the rows are the popup's inner height less borders and chrome.
        let popup = jinn_selection_widget::compute_popup_rect(frame);
        let expected = popup.height.saturating_sub(2).saturating_sub(3);
        assert_eq!(rows, expected as usize);
    }

    #[rstest::rstest]
    fn a_tiny_frame_still_reports_at_least_one_row() {
        // Given a terminal too small to fit the popup's chrome.
        let frame = Rect::new(0, 0, 8, 3);

        // When measuring the picker's results.
        let rows = results_viewport(frame);

        // Then paging has a non-zero window and cannot underflow.
        assert!(rows >= 1, "viewport must never be zero, got {rows}");
    }

    #[rstest::rstest]
    fn the_unmeasured_viewport_matches_the_fallback() {
        // Given a picker that has not rendered a frame yet.
        // When asking for its viewport to page against.
        let rows = unmeasured_viewport();

        // Then the documented fallback is used.
        assert_eq!(rows, RESULTS_VIEWPORT_FALLBACK);
    }
}
