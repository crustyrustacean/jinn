//! Terminal overlay geometry.
//!
//! The overlay is a centered rect over the whole frame with a small padding:
//! two columns on each side, one row top and bottom. [`terminal_overlay_rect`]
//! computes what the renderer draws into; [`terminal_overlay_inner_rect`]
//! subtracts the border and hint line to give the pty size (rows/cols), so a
//! program lays out for exactly the visible grid (WYSIWYG).

use ratatui::layout::Rect;

/// Horizontal padding on each side of the overlay.
pub const HORIZONTAL_PADDING: u16 = 2;
/// Vertical padding on the top and bottom of the overlay.
pub const VERTICAL_PADDING: u16 = 1;

/// Computes the overlay's outer rect: centered in `area` with small padding.
///
/// Always at least 1×1 so the renderer has somewhere to draw the
/// "no session" hint on tiny terminals.
#[must_use]
pub fn terminal_overlay_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(HORIZONTAL_PADDING * 2).max(1);
    let height = area.height.saturating_sub(VERTICAL_PADDING * 2).max(1);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Computes the overlay's inner rect — the pty size in `(rows, cols)` terms.
///
/// Subtracts the block border (1 cell each side) and the status/hint row
/// from the overlay rect.
#[must_use]
pub fn terminal_overlay_inner_rect(area: Rect) -> Rect {
    let overlay = terminal_overlay_rect(area);
    // Border: 1 cell each side; hint line: 1 row at the bottom.
    let width = overlay.width.saturating_sub(2).max(1);
    let height = overlay.height.saturating_sub(2 + 1).max(1);
    Rect {
        x: overlay.x + 1,
        y: overlay.y + 1,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn overlay_rect_is_centered_with_padding() {
        // Given an 80×24 frame.
        let area = Rect::new(0, 0, 80, 24);

        // When computing the overlay rect.
        let rect = terminal_overlay_rect(area);

        // Then it is inset by the padding (2 cols / 1 row each side).
        assert_eq!(rect.x, 2);
        assert_eq!(rect.y, 1);
        assert_eq!(rect.width, 76);
        assert_eq!(rect.height, 22);
    }

    #[rstest::rstest]
    fn inner_rect_subtracts_border_and_hint() {
        // Given an 80×24 frame.
        let area = Rect::new(0, 0, 80, 24);

        // When computing the pty-sized inner rect.
        let inner = terminal_overlay_inner_rect(area);

        // Then it is the overlay minus the border and the hint row.
        assert_eq!(inner.width, 74);
        assert_eq!(inner.height, 19);
    }

    #[rstest::rstest]
    fn tiny_frames_stay_in_bounds() {
        // Given a 1×1 frame.
        let area = Rect::new(0, 0, 1, 1);

        // When computing the overlay rect.
        let rect = terminal_overlay_rect(area);

        // Then it stays inside the frame.
        assert_eq!(rect.width, 1);
        assert_eq!(rect.height, 1);
        assert!(rect.x + rect.width <= area.width);
        assert!(rect.y + rect.height <= area.height);
    }
}
