//! Split border computation and rendering.
//!
//! Detects shared edges between areas produced by [`SplitManager`](ratatui_spatial_splits::SplitManager)
//! and draws box-drawing borders in a 1-unit gutter. The "first" child (left for
//! vertical splits, top for horizontal splits) is shrunk by 1 unit to make room
//! for the border line.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui_spatial_splits::{AreaId, SplitArea};

/// A border line between two adjacent split areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BorderLine {
    /// Orientation of the border.
    orientation: BorderOrientation,
    /// Position along the perpendicular axis (x for vertical, y for horizontal).
    pos: u16,
    /// Start of the span (inclusive) along the parallel axis.
    start: u16,
    /// End of the span (exclusive) along the parallel axis.
    end: u16,
}

/// Border orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BorderOrientation {
    /// Vertical border line (│), splits left/right areas.
    Vertical,
    /// Horizontal border line (─), splits top/bottom areas.
    Horizontal,
}

/// Result of border computation: adjusted rects and border lines.
pub(crate) struct BordersResult {
    /// Adjusted rects (shrunk to make room for borders) indexed by [`AreaId`].
    pub adjusted_rects: Vec<(AreaId, Rect)>,
    /// Border lines to render.
    pub lines: Vec<BorderLine>,
}

impl BordersResult {
    /// Returns the adjusted rect for the given area ID, or `None` if not found.
    pub fn rect_for(&self, id: AreaId) -> Option<Rect> {
        self.adjusted_rects
            .iter()
            .find(|(aid, _)| *aid == id)
            .map(|(_, rect)| *rect)
    }
}

/// Detects shared edges between areas and computes adjusted rects with border lines.
///
/// For each shared vertical edge, the left area shrinks by 1 column.
/// For each shared horizontal edge, the top area shrinks by 1 row.
/// Border lines are drawn in the freed gutter space.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounds-checked by surrounding logic"
)]
pub(crate) fn compute_split_borders(areas: &[SplitArea]) -> BordersResult {
    let mut adjusted: Vec<(AreaId, Rect)> = areas.iter().map(|a| (a.id, a.rect)).collect();
    let mut lines: Vec<BorderLine> = Vec::new();

    // Check all pairs for shared edges.
    for i in 0..areas.len() {
        for j in (i + 1)..areas.len() {
            let a = areas[i].rect;
            let b = areas[j].rect;

            // Check for vertical shared edge (areas side by side).
            if let Some(left_right) = check_vertical_edge(a, b) {
                let (left_id, left_rect, _right_rect) = if left_right {
                    (areas[i].id, a, b)
                } else {
                    (areas[j].id, b, a)
                };
                let right_rect = if left_right { b } else { a };

                let overlap_start = left_rect.y.max(right_rect.y);
                let overlap_end =
                    (left_rect.y + left_rect.height).min(right_rect.y + right_rect.height);

                if overlap_end > overlap_start {
                    lines.push(BorderLine {
                        orientation: BorderOrientation::Vertical,
                        pos: left_rect.x + left_rect.width - 1,
                        start: overlap_start,
                        end: overlap_end,
                    });

                    // Shrink left area by 1 column.
                    shrink_width(&mut adjusted, left_id);
                }
            }

            // Check for horizontal shared edge (areas stacked).
            if let Some(top_bottom) = check_horizontal_edge(a, b) {
                let (top_id, top_rect, _bottom_rect) = if top_bottom {
                    (areas[i].id, a, b)
                } else {
                    (areas[j].id, b, a)
                };
                let bottom_rect = if top_bottom { b } else { a };

                let overlap_start = top_rect.x.max(bottom_rect.x);
                let overlap_end =
                    (top_rect.x + top_rect.width).min(bottom_rect.x + bottom_rect.width);

                if overlap_end > overlap_start {
                    lines.push(BorderLine {
                        orientation: BorderOrientation::Horizontal,
                        pos: top_rect.y + top_rect.height - 1,
                        start: overlap_start,
                        end: overlap_end,
                    });

                    // Shrink top area by 1 row.
                    shrink_height(&mut adjusted, top_id);
                }
            }
        }
    }

    // Merge collinear segments at the same position into continuous lines.
    let lines = merge_lines(lines);

    BordersResult {
        adjusted_rects: adjusted,
        lines,
    }
}

/// Renders all border lines onto the frame buffer.
///
/// Detects junction points where vertical and horizontal lines intersect
/// and draws the appropriate box-drawing character (`┼`, `├`, `┤`, `┬`, `┴`).
pub(crate) fn render_borders(frame: &mut ratatui::Frame, lines: &[BorderLine]) {
    let buf = frame.buffer_mut();
    let style = Style::default().fg(Color::DarkGray);

    for (idx, line) in lines.iter().enumerate() {
        match line.orientation {
            BorderOrientation::Vertical => {
                for y in line.start..line.end {
                    let ch = junction_char(lines, idx, line.pos, y);
                    if let Some(cell) = buf.cell_mut((line.pos, y)) {
                        cell.set_symbol(ch);
                        cell.set_style(style);
                    }
                }
            }
            BorderOrientation::Horizontal => {
                for x in line.start..line.end {
                    let ch = junction_char(lines, idx, x, line.pos);
                    if let Some(cell) = buf.cell_mut((x, line.pos)) {
                        cell.set_symbol(ch);
                        cell.set_style(style);
                    }
                }
            }
        }
    }
}

/// Returns the appropriate box-drawing character at position (x, y).
///
/// Checks if any perpendicular border line crosses at this point.
/// If so, returns a junction or tee character based on which directions
/// the lines continue.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounds-checked by surrounding logic"
)]
fn junction_char(lines: &[BorderLine], current_idx: usize, x: u16, y: u16) -> &'static str {
    let current = &lines[current_idx];

    // Check for crossing perpendicular lines at (x, y).
    let mut up = false;
    let mut down = false;
    let mut left = false;
    let mut right = false;

    // Current line's contribution.
    match current.orientation {
        BorderOrientation::Vertical => {
            // This line is vertical at pos=x. It extends from start..end in y.
            if y > current.start {
                up = true;
            }
            if y + 1 < current.end {
                down = true;
            }
        }
        BorderOrientation::Horizontal => {
            // This line is horizontal at pos=y. It extends from start..end in x.
            if x > current.start {
                left = true;
            }
            if x + 1 < current.end {
                right = true;
            }
        }
    }

    // Check other lines for perpendicular crossings at (x, y).
    for (idx, other) in lines.iter().enumerate() {
        if idx == current_idx {
            continue;
        }

        match other.orientation {
            BorderOrientation::Vertical if current.orientation == BorderOrientation::Horizontal => {
                // Other is vertical at x=other.pos, spanning y in [start, end).
                if other.pos == x && y >= other.start && y < other.end {
                    if y > other.start {
                        up = true;
                    }
                    if y + 1 < other.end {
                        down = true;
                    }
                }
            }
            BorderOrientation::Horizontal if current.orientation == BorderOrientation::Vertical => {
                // Other is horizontal at y=other.pos, spanning x in [start, end).
                if other.pos == y && x >= other.start && x < other.end {
                    if x > other.start {
                        left = true;
                    }
                    if x + 1 < other.end {
                        right = true;
                    }
                }
            }
            _ => {}
        }
    }

    // No perpendicular crossing — just the straight line character.
    if !up && !down && !left && !right {
        return straight_char(current.orientation);
    }

    // Check if there's actually a perpendicular crossing (at least one
    // direction from the other axis must be present).
    match current.orientation {
        BorderOrientation::Vertical => {
            if !left && !right {
                // No horizontal line crosses here — straight vertical.
                return "│";
            }
        }
        BorderOrientation::Horizontal => {
            if !up && !down {
                // No vertical line crosses here — straight horizontal.
                return "─";
            }
        }
    }

    // Junction character based on directions.
    match (up, down, left, right) {
        (true, true, true, true) => "┼",
        (true, true, false, true) => "├",
        (true, true, true, false) => "┤",
        (false, true, true, true) => "┬",
        (true, false, true, true) => "┴",
        // Partial crossings (less common but handle gracefully).
        (true, true, false, false) => "│",
        (false, false, true, true) => "─",
        (true, false, false, true) => "┌",
        (true, false, true, false) => "┐",
        (false, true, false, true) => "└",
        (false, true, true, false) => "┘",
        // Single directions (endpoints) — shouldn't normally happen
        // but fall back to straight line.
        _ => straight_char(current.orientation),
    }
}

/// Returns the straight-line character for the given orientation.
fn straight_char(orientation: BorderOrientation) -> &'static str {
    match orientation {
        BorderOrientation::Vertical => "│",
        BorderOrientation::Horizontal => "─",
    }
}

// ── Line merging ─────────────────────────────────────────────

/// Merges overlapping or adjacent border lines at the same position and orientation.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounds-checked by surrounding logic"
)]
fn merge_lines(lines: Vec<BorderLine>) -> Vec<BorderLine> {
    if lines.is_empty() {
        return lines;
    }

    // Group by (orientation, pos).
    let mut groups: std::collections::HashMap<(BorderOrientation, u16), Vec<(u16, u16)>> =
        std::collections::HashMap::new();
    for line in &lines {
        groups
            .entry((line.orientation, line.pos))
            .or_default()
            .push((line.start, line.end));
    }

    let mut merged = Vec::new();
    for ((orientation, pos), mut spans) in groups {
        spans.sort_by_key(|(s, _)| *s);
        let mut current = spans[0];
        for (start, end) in spans.into_iter().skip(1) {
            if start <= current.1 {
                // Overlapping or adjacent — merge.
                current.1 = current.1.max(end);
            } else {
                merged.push(BorderLine {
                    orientation,
                    pos,
                    start: current.0,
                    end: current.1,
                });
                current = (start, end);
            }
        }
        merged.push(BorderLine {
            orientation,
            pos,
            start: current.0,
            end: current.1,
        });
    }

    merged
}

// ── Edge detection ─────────────────────────────────────────────

/// Check if two rects share a vertical edge (side by side).
///
/// Returns `Some(true)` if `a` is left of `b`, `Some(false)` if `b` is left of `a`,
/// or `None` if they don't share a vertical edge.
fn check_vertical_edge(a: Rect, b: Rect) -> Option<bool> {
    if a.x + a.width == b.x {
        Some(true) // a is left, b is right
    } else if b.x + b.width == a.x {
        Some(false) // b is left, a is right
    } else {
        None
    }
}

/// Check if two rects share a horizontal edge (stacked).
///
/// Returns `Some(true)` if `a` is above `b`, `Some(false)` if `b` is above `a`,
/// or `None` if they don't share a horizontal edge.
fn check_horizontal_edge(a: Rect, b: Rect) -> Option<bool> {
    if a.y + a.height == b.y {
        Some(true) // a is above b
    } else if b.y + b.height == a.y {
        Some(false) // b is above a
    } else {
        None
    }
}

/// Shrinks the width of the area with the given ID by 1.
fn shrink_width(adjusted: &mut [(AreaId, Rect)], id: AreaId) {
    for (aid, rect) in adjusted.iter_mut() {
        if *aid == id {
            rect.width = rect.width.saturating_sub(1);
            return;
        }
    }
}

/// Shrinks the height of the area with the given ID by 1.
fn shrink_height(adjusted: &mut [(AreaId, Rect)], id: AreaId) {
    for (aid, rect) in adjusted.iter_mut() {
        if *aid == id {
            rect.height = rect.height.saturating_sub(1);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a [`SplitArea`].
    fn area(id: u64, rect: Rect) -> SplitArea {
        SplitArea {
            id: AreaId(id),
            rect,
        }
    }

    // ── compute_split_borders tests ──────────────────────────────

    #[rstest::rstest]    fn vertical_split_produces_border() {
        // Given a vertical split: left(1) at (0,0,50,100), right(2) at (50,0,50,100).
        let areas = vec![
            area(1, Rect::new(0, 0, 50, 100)),
            area(2, Rect::new(50, 0, 50, 100)),
        ];

        // When computing borders.
        let result = compute_split_borders(&areas);

        // Then there is one vertical border at x=49 (the freed gutter column) spanning y=0..100.
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.lines[0].orientation, BorderOrientation::Vertical);
        assert_eq!(result.lines[0].pos, 49);
        assert_eq!(result.lines[0].start, 0);
        assert_eq!(result.lines[0].end, 100);
    }

    #[rstest::rstest]    fn vertical_split_shrinks_left() {
        // Given a vertical split: left(1) at (0,0,50,100), right(2) at (50,0,50,100).
        let areas = vec![
            area(1, Rect::new(0, 0, 50, 100)),
            area(2, Rect::new(50, 0, 50, 100)),
        ];

        // When computing borders.
        let result = compute_split_borders(&areas);

        // And the left area is shrunk by 1 column, right unchanged.
        assert_eq!(result.rect_for(AreaId(1)), Some(Rect::new(0, 0, 49, 100)));
        assert_eq!(result.rect_for(AreaId(2)), Some(Rect::new(50, 0, 50, 100)));
    }

    #[rstest::rstest]    fn horizontal_split_produces_border() {
        // Given a horizontal split: top(1) at (0,0,100,50), bottom(2) at (0,50,100,50).
        let areas = vec![
            area(1, Rect::new(0, 0, 100, 50)),
            area(2, Rect::new(0, 50, 100, 50)),
        ];

        // When computing borders.
        let result = compute_split_borders(&areas);

        // Then there is one horizontal border at y=49 (the freed gutter row) spanning x=0..100.
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.lines[0].orientation, BorderOrientation::Horizontal);
        assert_eq!(result.lines[0].pos, 49);
        assert_eq!(result.lines[0].start, 0);
        assert_eq!(result.lines[0].end, 100);
    }

    #[rstest::rstest]    fn horizontal_split_shrinks_top() {
        // Given a horizontal split: top(1) at (0,0,100,50), bottom(2) at (0,50,100,50).
        let areas = vec![
            area(1, Rect::new(0, 0, 100, 50)),
            area(2, Rect::new(0, 50, 100, 50)),
        ];

        // When computing borders.
        let result = compute_split_borders(&areas);

        // And the top area is shrunk by 1 row, bottom unchanged.
        assert_eq!(result.rect_for(AreaId(1)), Some(Rect::new(0, 0, 100, 49)));
        assert_eq!(result.rect_for(AreaId(2)), Some(Rect::new(0, 50, 100, 50)));
    }

    #[rstest::rstest]    fn grid_produces_two_border_lines() {
        // Given a 4-way grid layout.
        let areas = vec![
            area(1, Rect::new(0, 0, 50, 50)),   // top-left
            area(2, Rect::new(50, 0, 50, 50)),  // top-right
            area(3, Rect::new(0, 50, 50, 50)),  // bottom-left
            area(4, Rect::new(50, 50, 50, 50)), // bottom-right
        ];

        // When computing borders.
        let result = compute_split_borders(&areas);

        // Then there are 2 border lines after merging collinear segments:
        // - 1 vertical at x=49 spanning full height (0..100)
        // - 1 horizontal at y=49 spanning full width (0..100)
        assert_eq!(result.lines.len(), 2);

        let verticals: Vec<_> = result
            .lines
            .iter()
            .filter(|l| l.orientation == BorderOrientation::Vertical)
            .collect();
        let horizontals: Vec<_> = result
            .lines
            .iter()
            .filter(|l| l.orientation == BorderOrientation::Horizontal)
            .collect();

        assert_eq!(verticals.len(), 1);
        assert_eq!(verticals[0].pos, 49);
        assert_eq!(verticals[0].start, 0);
        assert_eq!(verticals[0].end, 100);

        assert_eq!(horizontals.len(), 1);
        assert_eq!(horizontals[0].pos, 49);
        assert_eq!(horizontals[0].start, 0);
        assert_eq!(horizontals[0].end, 100);
    }

    #[rstest::rstest]    fn top_left_shrunk_both_ways() {
        // Given a 4-way grid layout.
        let areas = vec![
            area(1, Rect::new(0, 0, 50, 50)),   // top-left
            area(2, Rect::new(50, 0, 50, 50)),  // top-right
            area(3, Rect::new(0, 50, 50, 50)),  // bottom-left
            area(4, Rect::new(50, 50, 50, 50)), // bottom-right
        ];

        // When computing borders.
        let result = compute_split_borders(&areas);

        // And top-left is shrunk both ways, top-right by 1 row,
        // bottom-left by 1 col, bottom-right unchanged.
        assert_eq!(result.rect_for(AreaId(1)), Some(Rect::new(0, 0, 49, 49)));
        assert_eq!(result.rect_for(AreaId(2)), Some(Rect::new(50, 0, 50, 49)));
        assert_eq!(result.rect_for(AreaId(3)), Some(Rect::new(0, 50, 49, 50)));
        assert_eq!(result.rect_for(AreaId(4)), Some(Rect::new(50, 50, 50, 50)));
    }

    #[rstest::rstest]    fn single_area_produces_no_borders() {
        // Given a single area.
        let areas = vec![area(1, Rect::new(0, 0, 100, 100))];

        // When computing borders.
        let result = compute_split_borders(&areas);

        // Then no borders and rect unchanged.
        assert!(result.lines.is_empty());
        assert_eq!(result.rect_for(AreaId(1)), Some(Rect::new(0, 0, 100, 100)));
    }

    // ── render_borders tests ─────────────────────────────────────

    #[rstest::rstest]    fn render_vertical_border_draws_line_character() {
        // Given a terminal buffer and a vertical border line.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let lines = vec![BorderLine {
            orientation: BorderOrientation::Vertical,
            pos: 49,
            start: 0,
            end: 20,
        }];

        // When rendering borders.
        terminal
            .draw(|frame| {
                render_borders(frame, &lines);
            })
            .unwrap();

        // Then the vertical border column contains │ characters.
        let buf = terminal.backend().buffer().clone();
        for y in 0..20u16 {
            let cell = buf.cell((49, y)).expect("cell");
            assert_eq!(cell.symbol(), "│");
            assert_eq!(cell.fg, Color::DarkGray);
        }
    }

    #[rstest::rstest]    fn render_horizontal_border_draws_dash_character() {
        // Given a terminal buffer and a horizontal border line.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let lines = vec![BorderLine {
            orientation: BorderOrientation::Horizontal,
            pos: 10,
            start: 0,
            end: 100,
        }];

        // When rendering borders.
        terminal
            .draw(|frame| {
                render_borders(frame, &lines);
            })
            .unwrap();

        // Then the horizontal border row contains ─ characters.
        let buf = terminal.backend().buffer().clone();
        for x in 0..100u16 {
            let cell = buf.cell((x, 10)).expect("cell");
            assert_eq!(cell.symbol(), "─");
            assert_eq!(cell.fg, Color::DarkGray);
        }
    }

    #[rstest::rstest]    fn crossing_point_is_cross_char() {
        // Given a 4-way grid with crossing borders.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(100, 100);
        let mut terminal = Terminal::new(backend).unwrap();

        let lines = vec![
            BorderLine {
                orientation: BorderOrientation::Vertical,
                pos: 50,
                start: 0,
                end: 100,
            },
            BorderLine {
                orientation: BorderOrientation::Horizontal,
                pos: 50,
                start: 0,
                end: 100,
            },
        ];

        // When rendering borders.
        terminal
            .draw(|frame| {
                render_borders(frame, &lines);
            })
            .unwrap();

        // Then the crossing point (50, 50) is ┼.
        let buf = terminal.backend().buffer().clone();
        let cross = buf.cell((50, 50)).expect("cross cell");
        assert_eq!(cross.symbol(), "┼");
    }

    #[rstest::rstest]    fn vertical_border_above_crossing() {
        // Given a 4-way grid with crossing borders.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(100, 100);
        let mut terminal = Terminal::new(backend).unwrap();

        let lines = vec![
            BorderLine {
                orientation: BorderOrientation::Vertical,
                pos: 50,
                start: 0,
                end: 100,
            },
            BorderLine {
                orientation: BorderOrientation::Horizontal,
                pos: 50,
                start: 0,
                end: 100,
            },
        ];

        // When rendering borders.
        terminal
            .draw(|frame| {
                render_borders(frame, &lines);
            })
            .unwrap();

        // And the vertical border above the crossing is │.
        let buf = terminal.backend().buffer().clone();
        assert_eq!(buf.cell((50, 0)).expect("above").symbol(), "│");
    }

    #[rstest::rstest]    fn horizontal_border_on_sides() {
        // Given a 4-way grid with crossing borders.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(100, 100);
        let mut terminal = Terminal::new(backend).unwrap();

        let lines = vec![
            BorderLine {
                orientation: BorderOrientation::Vertical,
                pos: 50,
                start: 0,
                end: 100,
            },
            BorderLine {
                orientation: BorderOrientation::Horizontal,
                pos: 50,
                start: 0,
                end: 100,
            },
        ];

        // When rendering borders.
        terminal
            .draw(|frame| {
                render_borders(frame, &lines);
            })
            .unwrap();

        // And the horizontal borders on each side are ─.
        let buf = terminal.backend().buffer().clone();
        assert_eq!(buf.cell((25, 50)).expect("left h").symbol(), "─");
        assert_eq!(buf.cell((75, 50)).expect("right h").symbol(), "─");
    }

    #[rstest::rstest]    fn render_asymmetric_split_produces_tee_junction() {
        // Given a layout where only one side is split horizontally:
        // left column is whole, right column split top/bottom.
        // Vertical border at x=50 spans full height.
        // Horizontal border at y=50 spans right half (50..100).
        // The junction at (50, 50) should be ├ (vertical continues, horizontal goes right).
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(100, 100);
        let mut terminal = Terminal::new(backend).unwrap();

        let lines = vec![
            BorderLine {
                orientation: BorderOrientation::Vertical,
                pos: 50,
                start: 0,
                end: 100,
            },
            BorderLine {
                orientation: BorderOrientation::Horizontal,
                pos: 50,
                start: 50,
                end: 100,
            },
        ];

        // When rendering borders.
        terminal
            .draw(|frame| {
                render_borders(frame, &lines);
            })
            .unwrap();

        // Then the junction at (50, 50) is ├ (vertical continues up/down, horizontal goes right).
        let buf = terminal.backend().buffer().clone();
        let tee = buf.cell((50, 50)).expect("tee cell");
        assert_eq!(tee.symbol(), "├");
    }
}
