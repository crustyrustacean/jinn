//! Mouse text selection state machine.
//!
//! Tracks the lifecycle of a user's click-and-drag text selection within a
//! constraining rectangular area (typically a UI pane). The state machine has
//! three states: `Idle` (no selection), `Dragging` (mouse button held), and
//! `Active` (selection finalized, awaiting clipboard copy).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_segmentation::UnicodeSegmentation as _;

/// Screen regions that support text selection, rebuilt each frame.
///
/// During rendering, the layout pushes `Rect`s for selectable areas (chat log,
/// picker popup, etc.). When a mouse click arrives, `find_for_position` returns
/// the most specific (smallest area) matching rect.
#[derive(Debug, Clone, Default)]
pub struct SelectableRects {
    /// The selectable regions.
    rects: Vec<Rect>,
}

impl SelectableRects {
    /// Creates an empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces all stored rects with a new set.
    pub fn rebuild(&mut self, rects: Vec<Rect>) {
        self.rects = rects;
    }

    /// Returns the smallest rect containing `(x, y)`, or `None`.
    ///
    /// "Smallest" means the rect with the least area — this picks the most
    /// specific pane when rects are nested (e.g. a popup inside the content area).
    /// Ties are broken by first-registered wins (stable iteration order).
    #[must_use]
    pub fn find_for_position(&self, x: u16, y: u16) -> Option<Rect> {
        self.rects
            .iter()
            .filter(|r| x >= r.x && x < r.right() && y >= r.y && y < r.bottom())
            .min_by_key(|r| r.width * r.height)
            .copied()
    }
}

/// Finds the x-position of the last non-whitespace cell in a row.
///
/// Scans from `x_max` backward to `x_min` (inclusive). Returns `None`
/// if no non-whitespace content is found in the range.
#[must_use]
pub(crate) fn find_last_nonws_in_row(
    buffer: &Buffer,
    y: u16,
    x_min: u16,
    x_max: u16,
) -> Option<u16> {
    (x_min..=x_max).rev().find(|&x| {
        buffer
            .cell((x, y))
            .is_some_and(|c| !c.symbol().chars().all(char::is_whitespace))
    })
}

/// The state of an in-progress or finalized mouse text selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SelectionState {
    /// No selection in progress.
    #[default]
    Idle,
    /// Mouse drag in progress.
    Dragging {
        /// Position where the drag started (screen coordinates).
        anchor: (u16, u16),
        /// Current drag position (screen coordinates), clamped to bounds.
        focus: (u16, u16),
        /// Constraining rectangle that selection is clipped to.
        bounds: Rect,
    },
    /// Finalized selection awaiting clipboard copy.
    Active {
        /// Position where the drag started (screen coordinates).
        anchor: (u16, u16),
        /// Position where the drag ended (screen coordinates).
        focus: (u16, u16),
        /// Constraining rectangle that selection is clipped to.
        bounds: Rect,
    },
}

impl SelectionState {
    /// Creates a new `Dragging` state starting at the given position.
    ///
    /// Both anchor and focus are initialized to `(x, y)`.
    pub fn start_drag(x: u16, y: u16, bounds: Rect) -> Self {
        Self::Dragging {
            anchor: (x, y),
            focus: (x, y),
            bounds,
        }
    }

    /// Updates the focus position during a drag, clamping to bounds.
    ///
    /// No-op for non-`Dragging` states (returns `self` unchanged).
    #[must_use]
    pub fn update_focus(self, x: u16, y: u16) -> Self {
        match self {
            Self::Dragging { anchor, bounds, .. } => {
                let clamped_x = x.clamp(bounds.x, bounds.right().saturating_sub(1));
                let clamped_y = y.clamp(bounds.y, bounds.bottom().saturating_sub(1));
                Self::Dragging {
                    anchor,
                    focus: (clamped_x, clamped_y),
                    bounds,
                }
            }
            other => other,
        }
    }

    /// Finalizes a `Dragging` selection into an `Active` one.
    ///
    /// No-op for non-`Dragging` states.
    #[must_use]
    pub fn finalize(self) -> Self {
        match self {
            Self::Dragging {
                anchor,
                focus,
                bounds,
            } => Self::Active {
                anchor,
                focus,
                bounds,
            },
            other => other,
        }
    }

    /// Cancels any selection, returning to `Idle`.
    #[must_use]
    pub fn cancel(self) -> Self {
        Self::Idle
    }

    /// Returns `true` if the state is anything other than `Idle`.
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Returns the anchor position, or `None` for `Idle`.
    #[must_use]
    pub fn anchor(&self) -> Option<(u16, u16)> {
        match self {
            Self::Idle => None,
            Self::Dragging { anchor, .. } | Self::Active { anchor, .. } => Some(*anchor),
        }
    }

    /// Returns the focus position, or `None` for `Idle`.
    #[must_use]
    pub fn focus(&self) -> Option<(u16, u16)> {
        match self {
            Self::Idle => None,
            Self::Dragging { focus, .. } | Self::Active { focus, .. } => Some(*focus),
        }
    }

    /// Returns the constraining bounds rect, or `None` for `Idle`.
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        match self {
            Self::Idle => None,
            Self::Dragging { bounds, .. } | Self::Active { bounds, .. } => Some(*bounds),
        }
    }

    /// Extracts the selected text from a ratatui buffer using line selection.
    ///
    /// Row classification:
    /// - **Single line** (anchor_y == focus_y): column-based from min(ax,fx) to max(ax,fx).
    /// - **First line** (anchor row): from anchor_x to last non-whitespace char.
    /// - **Middle lines**: from bounds.x to last non-whitespace char.
    /// - **Last line** (focus row): from bounds.x to focus_x.
    ///
    /// Trailing whitespace is trimmed per row. Rows are joined with `\n`.
    /// Empty trailing rows are omitted. Returns `None` for `Idle`.
    pub fn extract_text(&self, buffer: &Buffer) -> Option<String> {
        let (anchor, focus, bounds) = match self {
            Self::Dragging {
                anchor,
                focus,
                bounds,
            }
            | Self::Active {
                anchor,
                focus,
                bounds,
            } => (*anchor, *focus, *bounds),
            Self::Idle => return None,
        };

        let bounds_right = bounds.right().saturating_sub(1);
        let anchor_x = anchor.0.clamp(bounds.x, bounds_right);
        let focus_x = focus.0.clamp(bounds.x, bounds_right);
        let top_y = anchor.1.min(focus.1).max(bounds.y);
        let bot_y = anchor.1.max(focus.1).min(bounds_right);
        let bot_y = bot_y.min(bounds.bottom().saturating_sub(1));

        if top_y > bot_y {
            return Some(String::new());
        }

        let mut rows: Vec<String> = Vec::new();

        for y in top_y..=bot_y {
            let (start_x, end_x) = if top_y == bot_y {
                // Single line — column selection.
                (anchor_x.min(focus_x), anchor_x.max(focus_x))
            } else if y == anchor.1 {
                // First line — from anchor_x to last non-whitespace.
                let end =
                    find_last_nonws_in_row(buffer, y, anchor_x, bounds_right).unwrap_or(anchor_x);
                (anchor_x, end)
            } else if y == focus.1 {
                // Last line — from bounds.x to focus_x.
                (bounds.x, focus_x)
            } else {
                // Middle line — from bounds.x to last non-whitespace.
                let end =
                    find_last_nonws_in_row(buffer, y, bounds.x, bounds_right).unwrap_or(bounds.x);
                (bounds.x, end)
            };

            let mut row_symbols: Vec<String> = Vec::new();
            for x in start_x..=end_x {
                if let Some(cell) = buffer.cell((x, y)) {
                    row_symbols.push(cell.symbol().to_owned());
                }
            }
            let row_text = row_symbols.join("");
            // Trim trailing whitespace per row.
            let trimmed = row_text
                .graphemes(true)
                .collect::<Vec<_>>()
                .iter()
                .rev()
                .skip_while(|g| g.chars().all(char::is_whitespace))
                .copied()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            rows.push(trimmed);
        }

        // Strip empty trailing rows.
        while rows.last().is_some_and(std::string::String::is_empty) {
            rows.pop();
        }

        if rows.is_empty() {
            return Some(String::new());
        }

        Some(rows.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::new(0, 0, 20, 20)
    }

    #[rstest::rstest]
    fn start_drag_creates_dragging_state() {
        // Given no prior state.
        // When starting a drag at (5, 7) within bounds.
        let state = SelectionState::start_drag(5, 7, bounds());

        // Then the state is Dragging with anchor and focus at (5, 7).
        assert_eq!(
            state,
            SelectionState::Dragging {
                anchor: (5, 7),
                focus: (5, 7),
                bounds: bounds(),
            }
        );
    }

    #[rstest::rstest]
    fn update_focus_clamps_to_bounds() {
        // Given a Dragging state at (5, 5) with bounds (0, 0, 10, 10).
        let state = SelectionState::start_drag(5, 5, Rect::new(0, 0, 10, 10));

        // When updating focus to (15, 15) which exceeds bounds.
        let state = state.update_focus(15, 15);

        // Then the focus is clamped to (9, 9).
        assert_eq!(state.focus(), Some((9, 9)));
    }

    #[rstest::rstest]
    fn finalize_transitions_dragging_to_active() {
        // Given a Dragging state.
        let state = SelectionState::start_drag(1, 2, bounds()).update_focus(5, 6);

        // When finalizing.
        let state = state.finalize();

        // Then the state is Active with the same anchor, focus, and bounds.
        assert_eq!(
            state,
            SelectionState::Active {
                anchor: (1, 2),
                focus: (5, 6),
                bounds: bounds(),
            }
        );
    }

    #[rstest::rstest]
    fn cancel_returns_to_idle() {
        // Given a Dragging state.
        let state = SelectionState::start_drag(3, 4, bounds());

        // When cancelling.
        let state = state.cancel();

        // Then the state is Idle.
        assert_eq!(state, SelectionState::Idle);
    }

    #[rstest::rstest]
    fn idle_returns_none_for_accessors() {
        // Given an Idle state.
        let state = SelectionState::Idle;

        // Then all accessors return None.
        assert!(state.anchor().is_none());
        assert!(state.focus().is_none());
        assert!(state.bounds().is_none());
    }

    #[rstest::rstest]
    fn idle_returns_none_for_extract_text() {
        // Given an Idle state and an empty buffer.
        let state = SelectionState::Idle;
        let buffer = Buffer::empty(Rect::new(0, 0, 10, 5));

        // When extracting text.
        // Then it returns None.
        assert!(state.extract_text(&buffer).is_none());
    }

    #[rstest::rstest]
    fn single_row_selection_returns_text() {
        // Given a buffer with known text on row 2.
        let area = Rect::new(0, 0, 10, 5);
        let mut buffer = Buffer::empty(area);
        // Write "Hello" starting at (2, 2).
        for (i, ch) in "Hello".chars().enumerate() {
            let cell = buffer.cell_mut((2 + i as u16, 2)).unwrap();
            cell.set_symbol(&ch.to_string());
        }

        // And an Active selection covering cells (2,2) to (6,2).
        let state = SelectionState::Active {
            anchor: (2, 2),
            focus: (6, 2),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer);

        // Then text is returned (not None).
        assert!(text.is_some());
    }

    #[rstest::rstest]
    fn single_row_selection_returns_hello() {
        // Given a buffer with known text on row 2.
        let area = Rect::new(0, 0, 10, 5);
        let mut buffer = Buffer::empty(area);
        // Write "Hello" starting at (2, 2).
        for (i, ch) in "Hello".chars().enumerate() {
            let cell = buffer.cell_mut((2 + i as u16, 2)).unwrap();
            cell.set_symbol(&ch.to_string());
        }

        // And an Active selection covering cells (2,2) to (6,2).
        let state = SelectionState::Active {
            anchor: (2, 2),
            focus: (6, 2),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer).expect("should return text");

        // Then the text matches the content of those cells.
        assert_eq!(text, "Hello");
    }

    #[rstest::rstest]
    fn multi_row_selection_spans_rows() {
        // Given a buffer with text on two rows.
        let area = Rect::new(0, 0, 10, 5);
        let mut buffer = Buffer::empty(area);
        // Row 1: "AB" at (0, 1) and (1, 1).
        buffer.cell_mut((0, 1)).unwrap().set_symbol("A");
        buffer.cell_mut((1, 1)).unwrap().set_symbol("B");
        // Row 2: "CD" at (0, 2) and (1, 2).
        buffer.cell_mut((0, 2)).unwrap().set_symbol("C");
        buffer.cell_mut((1, 2)).unwrap().set_symbol("D");

        // And an Active selection spanning rows 1 and 2.
        let state = SelectionState::Active {
            anchor: (0, 1),
            focus: (1, 2),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer);

        // Then text is returned (not None).
        assert!(text.is_some());
    }

    #[rstest::rstest]
    fn rows_joined_with_newline() {
        // Given a buffer with text on two rows.
        let area = Rect::new(0, 0, 10, 5);
        let mut buffer = Buffer::empty(area);
        // Row 1: "AB" at (0, 1) and (1, 1).
        buffer.cell_mut((0, 1)).unwrap().set_symbol("A");
        buffer.cell_mut((1, 1)).unwrap().set_symbol("B");
        // Row 2: "CD" at (0, 2) and (1, 2).
        buffer.cell_mut((0, 2)).unwrap().set_symbol("C");
        buffer.cell_mut((1, 2)).unwrap().set_symbol("D");

        // And an Active selection spanning rows 1 and 2.
        let state = SelectionState::Active {
            anchor: (0, 1),
            focus: (1, 2),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer).expect("should return text");

        // Then the rows are joined with newline.
        assert_eq!(text, "AB\nCD");
    }

    #[rstest::rstest]
    fn accessors_return_positions_for_active_state() {
        // Given an Active state where anchor (5, 5) is after focus (2, 2).
        let state = SelectionState::Active {
            anchor: (5, 5),
            focus: (2, 2),
            bounds: bounds(),
        };

        // Then the accessors return the raw positions.
        assert_eq!(state.anchor(), Some((5, 5)));
        assert_eq!(state.focus(), Some((2, 2)));
        assert_eq!(state.bounds(), Some(bounds()));
    }

    // --- Line selection tests ---

    #[rstest::rstest]
    fn first_line_starts_at_anchor_x() {
        // Given a buffer with "ABCDE" on row 1 starting at col 0.
        let area = Rect::new(0, 0, 10, 5);
        let mut buffer = Buffer::empty(area);
        for (i, ch) in "ABCDE".chars().enumerate() {
            buffer
                .cell_mut((i as u16, 1))
                .unwrap()
                .set_symbol(&ch.to_string());
        }

        // And a 3-row selection starting at anchor (3, 1) going to focus (4, 3).
        let state = SelectionState::Active {
            anchor: (3, 1),
            focus: (4, 3),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer).expect("should return text");

        // Then the first line starts at anchor_x=3, so "DE".
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[0], "DE");
    }

    #[rstest::rstest]
    fn middle_line_selects_to_last_nonws() {
        // Given a buffer with "hello   world!" on row 2 (with spaces between).
        let area = Rect::new(0, 0, 15, 5);
        let mut buffer = Buffer::empty(area);
        for (i, ch) in "hello   world!".chars().enumerate() {
            buffer
                .cell_mut((i as u16, 2))
                .unwrap()
                .set_symbol(&ch.to_string());
        }

        // And a 3-row selection (rows 1-3), row 2 is a middle row.
        let state = SelectionState::Active {
            anchor: (0, 1),
            focus: (5, 3),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer).expect("should return text");

        // Then the middle row includes spaces between words.
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[1], "hello   world!");
    }

    #[rstest::rstest]
    fn last_line_stops_at_focus_x() {
        // Given a buffer with "ABCDEFGHIJ" on row 3.
        let area = Rect::new(0, 0, 15, 5);
        let mut buffer = Buffer::empty(area);
        for (i, ch) in "ABCDEFGHIJ".chars().enumerate() {
            buffer
                .cell_mut((i as u16, 3))
                .unwrap()
                .set_symbol(&ch.to_string());
        }

        // And a 3-row selection (rows 1-3), focus_x = 5.
        let state = SelectionState::Active {
            anchor: (0, 1),
            focus: (5, 3),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer).expect("should return text");

        // Then the last line stops at focus_x=5 (inclusive), so "ABCDEF".
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[2], "ABCDEF");
    }

    #[rstest::rstest]
    fn backward_selection_first_line_uses_anchor() {
        // Given a buffer with text on rows 1-3.
        let area = Rect::new(0, 0, 10, 5);
        let mut buffer = Buffer::empty(area);
        for (i, ch) in "ABCDE".chars().enumerate() {
            buffer
                .cell_mut((i as u16, 1))
                .unwrap()
                .set_symbol(&ch.to_string());
        }
        for (i, ch) in "FGHIJ".chars().enumerate() {
            buffer
                .cell_mut((i as u16, 2))
                .unwrap()
                .set_symbol(&ch.to_string());
        }
        for (i, ch) in "KLMNO".chars().enumerate() {
            buffer
                .cell_mut((i as u16, 3))
                .unwrap()
                .set_symbol(&ch.to_string());
        }

        // And a backward selection (anchor at row 3, focus at row 1).
        let state = SelectionState::Active {
            anchor: (3, 3),
            focus: (1, 1),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer).expect("should return text");

        // Then rows are iterated top to bottom:
        // y=1: focus.1 == 1, so last line: bounds.x=0 to focus_x=1 → "AB"
        // y=2: middle: bounds.x=0 to last_nonws → "FGHIJ"
        // y=3: anchor.1 == 3, so first line: anchor_x=3 to last_nonws → "NO"
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[0], "AB");
        assert_eq!(lines[1], "FGHIJ");
        assert_eq!(lines[2], "NO");
    }

    #[rstest::rstest]
    fn single_line_selection_is_column_based() {
        // Given a buffer with "ABCDE" on row 2.
        let area = Rect::new(0, 0, 10, 5);
        let mut buffer = Buffer::empty(area);
        for (i, ch) in "ABCDE".chars().enumerate() {
            buffer
                .cell_mut((i as u16, 2))
                .unwrap()
                .set_symbol(&ch.to_string());
        }

        // And a single-row selection from col 1 to col 3.
        let state = SelectionState::Active {
            anchor: (1, 2),
            focus: (3, 2),
            bounds: area,
        };

        // When extracting text.
        let text = state.extract_text(&buffer).expect("should return text");

        // Then only the columns between anchor_x and focus_x are selected.
        assert_eq!(text, "BCD");
    }

    // --- SelectableRects tests ---

    #[rstest::rstest]
    fn selectable_rects_find_returns_smallest_matching() {
        // Given overlapping rects — a large screen and a smaller pane.
        let screen = Rect::new(0, 0, 80, 24);
        let pane = Rect::new(10, 5, 20, 10);
        let mut rects = SelectableRects::new();
        rects.rebuild(vec![screen, pane]);

        // When querying a position inside the pane.
        let found = rects.find_for_position(15, 8);

        // Then the smaller pane rect is returned.
        assert_eq!(found, Some(pane));
    }

    #[rstest::rstest]
    fn selectable_rects_find_returns_none_for_position_outside_all() {
        // Given a single rect.
        let mut rects = SelectableRects::new();
        rects.rebuild(vec![Rect::new(0, 0, 10, 10)]);

        // When querying a position outside the rect.
        let found = rects.find_for_position(20, 20);

        // Then None is returned.
        assert_eq!(found, None);
    }

    #[rstest::rstest]
    fn selectable_rects_find_returns_none_when_empty() {
        // Given no rects registered.
        let rects = SelectableRects::new();

        // When querying any position.
        let found = rects.find_for_position(5, 5);

        // Then None is returned.
        assert_eq!(found, None);
    }

    #[rstest::rstest]
    fn selectable_rects_rebuild_replaces_previous_rects() {
        // Given rects with an initial rect.
        let mut rects = SelectableRects::new();
        rects.rebuild(vec![Rect::new(0, 0, 10, 10)]);

        // When rebuilding with different rects.
        rects.rebuild(vec![Rect::new(20, 20, 5, 5)]);

        // Then the old rect is gone and only the new one matches.
        assert_eq!(rects.find_for_position(5, 5), None);
        assert_eq!(
            rects.find_for_position(22, 22),
            Some(Rect::new(20, 20, 5, 5))
        );
    }
}
