//! Application layout computation and minimum size constants.

use ratatui::layout::{Constraint, Layout, Rect};

/// Minimum sidebar width in columns.
pub const MIN_SIDEBAR_WIDTH: u16 = 15;

/// Minimum terminal width.
pub const MIN_WIDTH: u16 = 40;
/// Minimum terminal height.
pub const MIN_HEIGHT: u16 = 14;

/// Top-level application layout areas.
pub struct AppLayout {
    /// The left column: chat + indicator + queue + input + status bar.
    pub main: Rect,
    /// The vertical minimap column (1 char wide, same height as chat log area).
    pub minimap: Rect,
    /// The right column: sidebar (full height).
    pub sidebar: Rect,
    /// The vertical border between minimap and sidebar (1 column wide).
    pub border: Rect,
    // Sub-areas of the main column:
    /// The content area (chat log + indicator + queue + bottom line).
    pub content: Rect,
    /// The input box area (dynamic height, chat tab only).
    pub input: Rect,
    /// The status bar area (1 row at very bottom).
    pub status_bar: Rect,
    /// The tab bar area (1 row at top of main area).
    pub tab_bar: Rect,
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
    /// main column | minimap(1) | border | sidebar (full height)
    ///   content   |   ▲        |        |
    ///             |   █        |        |
    ///             |   ▼        |        |
    ///   input     |            |        |
    ///   status    |            |        |
    /// ```
    ///
    /// `input_lines` is the number of visual lines the input box needs
    /// (used for dynamic multi-line input height).
    ///
    /// `max_input_height` caps the input box height (e.g., 50% of terminal).
    #[must_use]
    pub fn new(area: Rect, input_lines: u16, max_input_height: u16, sidebar_width: u16) -> Self {
        // Horizontal split: main column | minimap(1) | border(1) | sidebar
        let sidebar_width = sidebar_width
            .min(area.width.saturating_sub(MIN_WIDTH))
            .max(MIN_SIDEBAR_WIDTH);
        let border_width: u16 = 1;
        let minimap_width: u16 = 2;
        let main_width = area
            .width
            .saturating_sub(sidebar_width)
            .saturating_sub(border_width)
            .saturating_sub(minimap_width);

        let main = Rect {
            x: area.x,
            y: area.y,
            width: main_width,
            height: area.height,
        };

        let input_height = (1 + input_lines.max(1)).min(max_input_height);
        let [tab_bar, content, input, status_bar] = Layout::vertical([
            Constraint::Length(1), // tab bar
            Constraint::Min(1),
            Constraint::Length(input_height),
            Constraint::Length(2),
        ])
        .areas(main);

        // Minimap column: same height as the chat log area (content minus
        // the bottom indicator line and chat-bottom-line).
        let bottom_lines: u16 = 2;
        let chat_log_height = content.height.saturating_sub(bottom_lines);
        let minimap = Rect {
            x: main.x + main.width,
            y: content.y,
            width: minimap_width,
            height: chat_log_height,
        };

        let border = Rect {
            x: main.x + main.width + minimap_width,
            y: area.y,
            width: border_width,
            height: area.height,
        };
        let sidebar = Rect {
            x: main.x + main.width + minimap_width + border_width,
            y: area.y,
            width: sidebar_width,
            height: area.height,
        };

        Self {
            main,
            minimap,
            sidebar,
            border,
            content,
            input,
            status_bar,
            tab_bar,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test code, panics are acceptable"
    )]
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
        let layout = AppLayout::new(area, 1, area.height / 2, 30);

        // Then the status bar has height 1 and is at the bottom.
        assert_eq!(layout.status_bar.height, 2);
        assert!(layout.status_bar.y > layout.input.y);
        assert_eq!(layout.status_bar.y + layout.status_bar.height, area.height);
    }
}
