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
    /// - **Single line** (top_y == bot_y): column-based from min(ax,fx) to max(ax,fx).
    /// - **Top row**: from top_x to last non-whitespace char.
    /// - **Middle rows**: from bounds.x to last non-whitespace char.
    /// - **Bottom row**: from bounds.x to bot_x.
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

        // Resolve which point is on the top vs bottom row.
        let (top_x, bot_x) = if anchor.1 <= focus.1 {
            (anchor_x, focus_x)
        } else {
            (focus_x, anchor_x)
        };

        if top_y > bot_y {
            return Some(String::new());
        }

        let mut rows: Vec<String> = Vec::new();

        for y in top_y..=bot_y {
            let (start_x, end_x) = if top_y == bot_y {
                // Single line — column selection.
                (anchor_x.min(focus_x), anchor_x.max(focus_x))
            } else if y == top_y {
                // Top row — from top_x to last non-whitespace.
                let end = find_last_nonws_in_row(buffer, y, top_x, bounds_right).unwrap_or(top_x);
                (top_x, end)
            } else if y == bot_y {
                // Bottom row — from bounds.x to bot_x.
                (bounds.x, bot_x)
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
