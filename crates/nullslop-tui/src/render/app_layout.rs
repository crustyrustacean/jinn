//! Application layout computation and minimum size constants.

use ratatui::layout::{Constraint, Layout, Rect};

/// Minimum terminal width.
pub const MIN_WIDTH: u16 = 40;
/// Minimum terminal height.
pub const MIN_HEIGHT: u16 = 14;

/// Top-level application layout areas.
pub struct AppLayout {
    /// The tab bar area (1 row at top, full width).
    pub tabs: Rect,
    /// The left column: chat + indicator + queue + input + status bar.
    pub main: Rect,
    /// The right column: sidebar (full height below tabs).
    pub sidebar: Rect,
    /// The vertical border between main and sidebar (1 column wide).
    pub border: Rect,
    // Sub-areas of the main column:
    /// The content area (chat log + indicator + queue + bottom line).
    pub content: Rect,
    /// The input box area (dynamic height, chat tab only).
    pub input: Rect,
    /// The status bar area (1 row at very bottom).
    pub status_bar: Rect,
}

impl AppLayout {
    /// Returns `true` if the given area meets minimum size requirements.
    #[must_use]
    pub const fn meets_min_size(area: Rect) -> bool {
        area.width >= MIN_WIDTH && area.height >= MIN_HEIGHT
    }

    /// Computes the layout for the given terminal area.
    ///
    /// Layout structure:
    /// ```text
    /// tabs (full width)
    /// main column | border | sidebar (full height)
    ///   content   |        |
    ///   input     |        |
    ///   status    |        |
    /// ```
    ///
    /// `input_lines` is the number of visual lines the input box needs
    /// (used for dynamic multi-line input height).
    ///
    /// `max_input_height` caps the input box height (e.g., 50% of terminal).
    #[must_use]
    pub fn new(area: Rect, input_lines: u16, max_input_height: u16) -> Self {
        let [tabs, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

        // Horizontal split: main column | border(1) | sidebar
        let sidebar_width = (rest.width * 20 / 100).min(30);
        let border_width: u16 = 1;
        let main_width = rest
            .width
            .saturating_sub(sidebar_width)
            .saturating_sub(border_width);

        let main = Rect {
            x: rest.x,
            y: rest.y,
            width: main_width,
            height: rest.height,
        };
        let border = Rect {
            x: rest.x + main_width,
            y: rest.y,
            width: border_width,
            height: rest.height,
        };
        let sidebar = Rect {
            x: rest.x + main_width + border_width,
            y: rest.y,
            width: sidebar_width,
            height: rest.height,
        };

        let input_height = (1 + input_lines.max(1)).min(max_input_height);
        let [content, input, status_bar] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(input_height),
            Constraint::Length(2),
        ])
        .areas(main);

        Self {
            tabs,
            main,
            sidebar,
            border,
            content,
            input,
            status_bar,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[rstest::rstest]
    fn meets_min_size() {
        // Given a 40x14 area.
        let area = Rect::new(0, 0, 40, 14);

        // When checking meets_min_size.
        let result = AppLayout::meets_min_size(area);

        // Then it returns true.
        assert!(result);
    }

    #[rstest::rstest]
    fn too_small() {
        // Given a 10x5 area.
        let area = Rect::new(0, 0, 10, 5);

        // When checking meets_min_size.
        let result = AppLayout::meets_min_size(area);

        // Then it returns false.
        assert!(!result);
    }

    #[rstest::rstest]
    fn includes_status_bar() {
        // Given a 40x14 area.
        let area = Rect::new(0, 0, 40, 14);
        let layout = AppLayout::new(area, 1, area.height / 2);

        // Then the status bar has height 1 and is at the bottom.
        assert_eq!(layout.status_bar.height, 2);
        assert!(layout.status_bar.y > layout.input.y);
        assert_eq!(layout.status_bar.y + layout.status_bar.height, area.height);
    }
}
