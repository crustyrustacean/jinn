//! The session-lifecycle picker's viewport measurement.
//!
//! Paging (`PgUp`/`PgDn`) needs to
//! know how many result rows are actually on screen: the selection widget keeps
//! the highlighted row inside a visible window of that size, and a wrong value
//! lets the highlight scroll out of sight.
//!
//! The kernel used to publish this measurement into its own state once per
//! frame, and a picker in kernel scope could read it back. A slice-owned picker
//! has no such state, so it measures in its own renderer instead — same layout
//! arithmetic, same numbers, no shared field.

use ratatui::layout::Rect;

/// Input + separator rows the list pane reserves above its results, in the
/// side-by-side preview layout.
const CHROME_ROWS_LIST: u16 = 2;

/// The session-lifecycle picker's result-row count for the given popup rectangle.
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
    #[case::wide(Rect::new(0, 0, 200, 50))]
    #[case::narrow(Rect::new(0, 0, 80, 24))]
    #[case::tiny(Rect::new(0, 0, 20, 8))]
    fn the_viewport_is_never_zero(#[case] area: Rect) {
        // Given the popup rectangle this frame size produces.
        let popup = jinn_selection_widget::compute_popup_rect(area);

        // When the result-row count is measured.
        let rows = results_viewport(popup);

        // Then it is at least 1, so paging never windows against nothing.
        assert!(rows >= 1, "viewport must never be zero, got {rows}");
    }

    #[rstest::rstest]
    fn the_unmeasured_viewport_matches_the_fallback() {
        // Given a picker that has not rendered a frame yet.
        // When asking for its viewport to page against.
        let rows = jinn_session_lifecycle_msg::RESULTS_VIEWPORT_FALLBACK;

        // Then the documented fallback is what the un-rendered cell holds.
        assert_eq!(
            jinn_session_lifecycle_msg::SessionLifecyclePickerState::default().results_viewport,
            rows
        );
    }
}
