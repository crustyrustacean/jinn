//! The stream-rule picker's viewport measurement.
//!
//! Paging (`PgUp`/`PgDn`) needs to know how many result rows are actually
//! on screen: the selection widget keeps the highlighted row inside a
//! visible window of that size, and a wrong value lets the highlight
//! scroll out of sight. Same layout arithmetic as the sibling pickers —
//! the picker measures in its own renderer instead of sharing a field.

use ratatui::layout::Rect;

/// Input + separator rows the list pane reserves above its results.
const CHROME_ROWS: u16 = 2;

/// The stream-rule picker's result-row count for the given popup rectangle.
///
/// `popup` is the popup rectangle handed down by the render pass (see
/// `jinn_selection_widget::compute_popup_rect`), not a terminal frame.
/// At least 1, so paging never windows against zero on a tiny terminal.
#[must_use]
pub fn results_viewport(popup: Rect) -> usize {
    let inner = popup.height.saturating_sub(2); // both borders

    // The picker draws the plain selection widget: no preview split, so the
    // list pane spans the full inner height less its input and separator,
    // whatever the popup's width.
    inner.saturating_sub(CHROME_ROWS).max(1) as usize
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test module, panics are acceptable")]

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
    fn a_tall_popup_measures_its_inner_height_less_the_chrome() {
        // Given a popup tall enough for rows plus chrome.
        let popup = Rect::new(0, 0, 100, 20);

        // When measuring the picker's results.
        let rows = results_viewport(popup);

        // Then the count is the inner height less both borders and the
        // input + separator rows.
        assert_eq!(rows, (20 - 2 - CHROME_ROWS) as usize);
    }

    #[rstest::rstest]
    fn the_unmeasured_viewport_matches_the_fallback() {
        // Given a picker that has not rendered a frame yet.
        // When asking for its viewport to page against.
        // Then the documented fallback is what the un-rendered cell holds.
        assert_eq!(
            jinn_stream_rule_picker_msg::StreamRulePickerState::default().results_viewport,
            jinn_stream_rule_picker_msg::RESULTS_VIEWPORT_FALLBACK
        );
    }
}
