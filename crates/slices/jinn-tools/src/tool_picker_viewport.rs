// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The tool picker's viewport measurement.
//!
//! Paging (`PgUp`/`PgDn`) and the toggle's advance-to-next-row both need to
//! know how many result rows are actually on screen: the selection widget keeps
//! the highlighted row inside a visible window of that size, and a wrong value
//! lets the highlight scroll out of sight.
//!
//! The kernel used to publish this measurement into its own state once per
//! frame, and a picker in kernel scope could read it back. A slice-owned picker
//! has no such state, so it measures in its own renderer instead — same layout
//! arithmetic, same numbers, no shared field.
//!
//! The tool picker is a plain list (no preview pane), so the arithmetic is the
//! widget's standard split: the popup's inner height, less the filter box and
//! separator above the results, less the two footer rows (the live enabled
//! count and the keybind line).

use ratatui::layout::Rect;

/// Filter + separator rows the list pane reserves above its results.
const CHROME_ROWS_LIST: u16 = 2;

/// Footer rows the tool picker draws beneath its results: the status line and
/// the keybind line.
const FOOTER_ROWS: u16 = 2;

/// The tool picker's result-row count for the given popup rectangle.
///
/// Mirrors the widget's own layout so the value tracks what the user can see.
/// At least 1, so paging never windows against zero on a tiny terminal.
#[must_use]
pub fn results_viewport(popup: Rect) -> usize {
    let inner = popup.height.saturating_sub(2); // both borders
    inner
        .saturating_sub(CHROME_ROWS_LIST)
        .saturating_sub(FOOTER_ROWS)
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
    fn the_measurement_reserves_the_filter_chrome_and_both_footer_rows() {
        // Given a frame tall enough to lay the whole popup out.
        let frame = Rect::new(0, 0, 100, 30);

        // When measuring the picker's results.
        let popup = jinn_selection_widget::compute_popup_rect(frame);
        let rows = results_viewport(popup);

        // Then the rows are the popup's inner height less the borders, the
        // filter chrome, and the two footer rows.
        let expected = popup
            .height
            .saturating_sub(2)
            .saturating_sub(CHROME_ROWS_LIST)
            .saturating_sub(FOOTER_ROWS);
        assert_eq!(rows, expected as usize);
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
