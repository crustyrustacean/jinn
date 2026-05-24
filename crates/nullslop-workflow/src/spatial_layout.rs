//! Spatial layout computation for workflow nodes.
//!
//! Provides pure-math functions for computing node dimensions and 2D positions
//! from a [`WorkflowStructure`]. Shared between the TUI renderer and the domain
//! crate's spatial navigation logic.
//!
//! # Layout algorithm
//!
//! Nodes are assigned to columns based on topological depth (source = column 0).
//! Within each column, nodes are stacked vertically. Column widths are determined
//! by the widest node in the preceding columns, with horizontal spacing.

use std::collections::HashMap;

use crate::port::PortDef;
use crate::execution::WorkflowStructure;

/// A bounding rectangle in content-space coordinates.
///
/// Represents the position and size of a workflow node as rendered
/// in the TUI. Coordinates are cell-based (0-indexed from top-left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpatialRect {
    /// Left edge X position.
    pub x: u16,
    /// Top edge Y position.
    pub y: u16,
    /// Width in cells.
    pub width: u16,
    /// Height in cells.
    pub height: u16,
}

impl SpatialRect {
    /// Center X coordinate.
    #[must_use]
    pub fn center_x(&self) -> u16 {
        self.x + self.width / 2
    }

    /// Center Y coordinate.
    #[must_use]
    pub fn center_y(&self) -> u16 {
        self.y + self.height / 2
    }

    /// Right edge X position.
    #[must_use]
    pub fn right(&self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// Bottom edge Y position.
    #[must_use]
    pub fn bottom(&self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// Whether this rect's horizontal range overlaps another's.
    #[must_use]
    pub fn overlaps_x(&self, other: &SpatialRect) -> bool {
        self.x < other.right() && other.x < self.right()
    }

    /// Whether this rect's vertical range overlaps another's.
    #[must_use]
    pub fn overlaps_y(&self, other: &SpatialRect) -> bool {
        self.y < other.bottom() && other.y < self.bottom()
    }
}

/// Direction of spatial navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialDirection {
    /// Move left.
    Left,
    /// Move down.
    Down,
    /// Move up.
    Up,
    /// Move right.
    Right,
}

/// Horizontal padding inside the node border (space between border and content).
const H_PAD: usize = 1;

/// Minimum content width (prevents tiny boxes).
const MIN_CONTENT_WIDTH: usize = 4;

/// Horizontal spacing between columns (cells).
const H_SPACING: u16 = 5;

/// Vertical spacing between nodes in the same column (cells).
const V_SPACING: u16 = 1;

/// Computes the width and height of a node box from its name and port definitions.
///
/// This is the pure-math portion of `VisualNode::compute()`, extracted so both
/// the TUI renderer and the spatial navigation logic use identical dimensions.
///
/// Width = `max(title, longest port label, MIN_CONTENT_WIDTH) + 2 borders + 2 padding`.
///
/// Height = `1 + max(inputs, 1) + 1 + max(outputs, 1) + 1`.
#[must_use]
pub fn compute_node_size(name: &str, inputs: &[PortDef], outputs: &[PortDef]) -> (u16, u16) {
    let title_width = name.len() + 1; // +1 for status indicator space

    let port_width = inputs
        .iter()
        .chain(outputs.iter())
        .map(|def| {
            let type_str = def.value_type.label();
            type_str.len() + 1 + def.name.len() // "Type name"
        })
        .max()
        .unwrap_or(0);

    let content_width = title_width.max(port_width).max(MIN_CONTENT_WIDTH);
    let width = u16::try_from(content_width + 2 + 2 * H_PAD).unwrap_or(u16::MAX);

    let input_count = inputs.len().max(1);
    let output_count = outputs.len().max(1);
    // height = top_border + inputs + gap + outputs + bottom_border
    let height = u16::try_from(1 + input_count + 1 + output_count + 1).unwrap_or(u16::MAX);

    (width, height)
}

/// Computes the 2D layout for all nodes in a workflow graph.
///
/// Returns a map from node name to its bounding rectangle in content-space
/// coordinates. Positions are deterministic from the graph topology.
///
/// The algorithm mirrors `nullslop_workflow_tui::layout::compute()` exactly:
/// 1. Assign each node a topological column.
/// 2. Compute node sizes from port definitions.
/// 3. Stack nodes vertically within each column.
/// 4. Offset each column horizontally based on preceding column widths.
///
/// # Panics
///
/// Does not panic; all internal lookups are guaranteed by construction.
#[must_use]
pub fn compute_spatial_layout(structure: &WorkflowStructure) -> HashMap<String, SpatialRect> {
    let all_names: Vec<&str> = structure.node_names().collect();
    if all_names.is_empty() {
        return HashMap::new();
    }

    let columns = compute_columns(structure);

    let mut column_nodes: HashMap<usize, Vec<&str>> = HashMap::new();
    for name in &all_names {
        let col = columns.get(*name).copied().unwrap_or(0);
        column_nodes.entry(col).or_default().push(name);
    }

    let max_col = columns.values().copied().max().unwrap_or(0);

    // First pass: compute all node sizes.
    let mut node_sizes: HashMap<&str, (u16, u16)> = HashMap::new();
    for name in &all_names {
        let input_defs = structure.node_input_ports(name).unwrap_or_default();
        let output_defs = structure.node_output_ports(name).unwrap_or_default();
        let (w, h) = compute_node_size(name, input_defs, output_defs);
        node_sizes.insert(name, (w, h));
    }

    // Second pass: assign positions by column.
    let mut result = HashMap::new();
    for col in 0..=max_col {
        let Some(col_names) = column_nodes.get(&col) else {
            continue;
        };

        let x_offset = compute_x_offset(&node_sizes, &column_nodes, col);

        let mut y_cursor: u16 = 0;
        for name in col_names {
            #[expect(clippy::expect_used, reason = "size was computed in first pass")]
            let &(width, height) = node_sizes
                .get(name)
                .expect("size computed in first pass");
            result.insert(
                (*name).to_owned(),
                SpatialRect {
                    x: x_offset,
                    y: y_cursor,
                    width,
                    height,
                },
            );
            y_cursor = y_cursor + height + V_SPACING;
        }
    }

    result
}

/// Computes the topological column for each node.
///
/// Source nodes get column 0. Every other node gets `max(parent_columns) + 1`.
fn compute_columns(structure: &WorkflowStructure) -> HashMap<&str, usize> {
    let mut columns: HashMap<&str, usize> = HashMap::new();

    // Initialize source nodes at column 0.
    for name in structure.sources() {
        columns.insert(name.as_str(), 0);
    }

    // Also handle nodes with no incoming edges that aren't in sources().
    for name in structure.node_names() {
        let has_inputs = structure
            .node_input_ports(name)
            .is_some_and(|ports| !ports.is_empty());
        if !has_inputs {
            columns.insert(name, 0);
        }
    }

    // Propagate: for each edge, target column = max(target column, source column + 1).
    let mut changed = true;
    while changed {
        changed = false;
        for edge in structure.edges() {
            let src_col = columns.get(edge.source_node.as_str()).copied().unwrap_or(0);
            let tgt_col = columns.get(edge.target_node.as_str()).copied().unwrap_or(0);
            let new_col = src_col + 1;
            if new_col > tgt_col {
                columns.insert(edge.target_node.as_str(), new_col);
                changed = true;
            }
        }
    }

    // Assign column 0 to any remaining nodes (disconnected, no edges).
    for name in structure.node_names() {
        columns.entry(name).or_insert(0);
    }

    columns
}

/// Computes the x offset for a given column.
fn compute_x_offset(
    node_sizes: &HashMap<&str, (u16, u16)>,
    column_nodes: &HashMap<usize, Vec<&str>>,
    target_col: usize,
) -> u16 {
    let mut x: u16 = 0;
    for col in 0..target_col {
        let max_width = column_nodes
            .get(&col)
            .map_or(0, |names| {
                names
                    .iter()
                    .filter_map(|n| node_sizes.get(n).map(|(w, _)| *w))
                    .max()
                    .unwrap_or(0)
            });
        x = x + max_width + H_SPACING;
    }
    x
}

/// Given the current node's rect and all candidate rects, find the nearest
/// node in the given direction.
///
/// # Algorithm
///
/// 1. Filter out the current node.
/// 2. For the primary axis (e.g., Y for Down), only consider candidates
///    with a positive delta in that direction.
/// 3. Prefer candidates whose cross-axis range overlaps the current node's
///    cross-axis range.
/// 4. Among those, pick the one with the smallest primary-axis delta.
///    Ties broken by smallest cross-axis delta.
/// 5. If no overlapping candidates exist, fall back to all candidates in
///    the primary direction, but require the primary-axis delta ≥ cross-axis
///    delta (angle constraint — not too steep).
/// 6. Among those, pick by smallest primary-axis delta, then cross-axis.
/// 7. If still nothing, return `None` (no-op).
#[must_use]
pub fn spatial_nearest(
    current: &SpatialRect,
    direction: SpatialDirection,
    candidates: &HashMap<String, SpatialRect>,
    current_name: &str,
) -> Option<String> {
    type RectGetter = fn(&SpatialRect) -> u16;
    let (primary, cross, sign_positive): (RectGetter, RectGetter, bool) =
        match direction {
            SpatialDirection::Down => (SpatialRect::center_y, SpatialRect::center_x, true),
            SpatialDirection::Up => (SpatialRect::center_y, SpatialRect::center_x, false),
            SpatialDirection::Right => (SpatialRect::center_x, SpatialRect::center_y, true),
            SpatialDirection::Left => (SpatialRect::center_x, SpatialRect::center_y, false),
        };

    let cur_primary = primary(current);
    let cur_cross = cross(current);

    let overlap_check: fn(&SpatialRect, &SpatialRect) -> bool = match direction {
        SpatialDirection::Down | SpatialDirection::Up => SpatialRect::overlaps_x,
        SpatialDirection::Left | SpatialDirection::Right => SpatialRect::overlaps_y,
    };

    // Collect candidates with their deltas.
    let mut scored: Vec<(String, i32, i32, bool)> = Vec::new();

    for (name, rect) in candidates {
        if name == current_name {
            continue;
        }

        let other_primary = primary(rect);
        let delta_primary = i32::from(other_primary) - i32::from(cur_primary);

        // Filter by direction.
        if sign_positive && delta_primary <= 0 {
            continue;
        }
        if !sign_positive && delta_primary >= 0 {
            continue;
        }

        let other_cross = cross(rect);
        let delta_cross = (i32::from(other_cross) - i32::from(cur_cross)).abs();

        let overlaps = overlap_check(current, rect);

        scored.push((name.clone(), delta_primary, delta_cross, overlaps));
    }

    if scored.is_empty() {
        return None;
    }

    // Try overlapping candidates first.
    let overlap_candidates: Vec<_> = scored
        .iter()
        .filter(|(_, _, _, overlaps)| *overlaps)
        .collect();

    if let Some(best) = pick_nearest(&overlap_candidates) {
        return Some(best.clone());
    }

    // Fallback: diagonal candidates with angle constraint.
    // For vertical moves: primary = Y, cross = X. Require |delta_y| >= |delta_x|.
    // For horizontal moves: primary = X, cross = Y. Require |delta_x| >= |delta_y|.
    let diagonal_candidates: Vec<_> = scored
        .iter()
        .filter(|(_, delta_primary, delta_cross, _)| delta_primary.abs() >= *delta_cross)
        .collect();

    if let Some(best) = pick_nearest(&diagonal_candidates) {
        return Some(best.clone());
    }

    None
}

/// Picks the candidate with smallest primary delta, then smallest cross delta.
fn pick_nearest<'a>(candidates: &'a [&'a (String, i32, i32, bool)]) -> Option<&'a String> {
    candidates
        .iter()
        .min_by(|a, b| {
            a.1.abs()
                .cmp(&b.1.abs())
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.0.cmp(&b.0))
        })
        .map(|(name, _, _, _)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SpatialRect tests ---

    #[test]
    fn spatial_rect_center() {
        let rect = SpatialRect {
            x: 10,
            y: 20,
            width: 6,
            height: 8,
        };
        assert_eq!(rect.center_x(), 13);
        assert_eq!(rect.center_y(), 24);
    }

    #[test]
    fn spatial_rect_edges() {
        let rect = SpatialRect {
            x: 5,
            y: 10,
            width: 20,
            height: 5,
        };
        assert_eq!(rect.right(), 25);
        assert_eq!(rect.bottom(), 15);
    }

    #[test]
    fn spatial_rect_overlaps_x_touching_not_overlapping() {
        let a = SpatialRect {
            x: 0,
            y: 0,
            width: 10,
            height: 5,
        };
        let b = SpatialRect {
            x: 10,
            y: 0,
            width: 10,
            height: 5,
        };
        // a.right() == 10, b.x == 10, so a.x < b.right() but NOT a.x < b.right() fails
        // Actually: overlaps_x: self.x < other.right() && other.x < self.right()
        // a: x=0, right=10. b: x=10, right=20.
        // 0 < 20 && 10 < 10 → false
        assert!(!a.overlaps_x(&b));
    }

    #[test]
    fn spatial_rect_overlaps_x_partial() {
        let a = SpatialRect {
            x: 0,
            y: 0,
            width: 15,
            height: 5,
        };
        let b = SpatialRect {
            x: 10,
            y: 0,
            width: 10,
            height: 5,
        };
        // 0 < 20 && 10 < 15 → true
        assert!(a.overlaps_x(&b));
    }

    #[test]
    fn spatial_rect_overlaps_y() {
        let a = SpatialRect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let b = SpatialRect {
            x: 0,
            y: 5,
            width: 10,
            height: 10,
        };
        assert!(a.overlaps_y(&b));
    }

    // --- compute_node_size tests ---

    #[test]
    fn node_size_simple_passthrough() {
        let inputs = [PortDef::text("in")];
        let outputs = [PortDef::text("out")];
        let (w, h) = compute_node_size("node", &inputs, &outputs);

        // content_width = max("node ".len()=5, max("Text in".len()=7, "Text out".len()=8), 4) = 8
        // width = 8 + 2 + 2 = 12
        assert_eq!(w, 12);
        // height = 1 + 1 + 1 + 1 + 1 = 5
        assert_eq!(h, 5);
    }

    #[test]
    fn node_size_source() {
        let outputs = [PortDef::text("out")];
        let (w, h) = compute_node_size("source", &[], &outputs);

        // content_width = max("source ".len()=7, "Text out".len()=8, 4) = 8
        // width = 8 + 2 + 2 = 12
        assert_eq!(w, 12);
        // height = 1 + max(0,1) + 1 + 1 + 1 = 5
        assert_eq!(h, 5);
    }

    #[test]
    fn node_size_sink() {
        let inputs = [PortDef::text("in")];
        let (w, h) = compute_node_size("sink", &inputs, &[]);

        // content_width = max("sink ".len()=5, "Text in".len()=7, 4) = 7
        // width = 7 + 2 + 2 = 11
        assert_eq!(w, 11);
        // height = 1 + 1 + 1 + max(0,1) + 1 = 5
        assert_eq!(h, 5);
    }

    #[test]
    fn node_size_merge_two_inputs() {
        let inputs = [PortDef::text("in_1"), PortDef::text("in_2")];
        let (w, h) = compute_node_size("merge", &inputs, &[]);

        // content_width = max("merge ".len()=6, max("Text in_1".len()=9, "Text in_2".len()=9), 4) = 9
        // width = 9 + 2 + 2 = 13
        assert_eq!(w, 13);
        // height = 1 + 2 + 1 + 1 + 1 = 6
        assert_eq!(h, 6);
    }

    // --- spatial_nearest tests ---

    fn make_rect(name: &str, x: u16, y: u16, w: u16, h: u16) -> (String, SpatialRect) {
        (
            name.to_owned(),
            SpatialRect {
                x,
                y,
                width: w,
                height: h,
            },
        )
    }

    #[test]
    fn nearest_down_returns_node_below_in_same_column() {
        let a = make_rect("a", 0, 0, 10, 5);
        let b = make_rect("b", 0, 6, 10, 5);
        let c = make_rect("c", 0, 12, 10, 5);

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);
        candidates.insert(b.0.clone(), b.1);
        candidates.insert(c.0.clone(), c.1);

        // From a, Down → b
        let result = spatial_nearest(&a.1, SpatialDirection::Down, &candidates, "a");
        assert_eq!(result.as_deref(), Some("b"));

        // From b, Down → c
        let result = spatial_nearest(&b.1, SpatialDirection::Down, &candidates, "b");
        assert_eq!(result.as_deref(), Some("c"));
    }

    #[test]
    fn nearest_up_returns_node_above_in_same_column() {
        let a = make_rect("a", 0, 0, 10, 5);
        let b = make_rect("b", 0, 6, 10, 5);

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);
        candidates.insert(b.0.clone(), b.1);

        // From b, Up → a
        let result = spatial_nearest(&b.1, SpatialDirection::Up, &candidates, "b");
        assert_eq!(result.as_deref(), Some("a"));
    }

    #[test]
    fn nearest_right_returns_node_in_adjacent_column() {
        let a = make_rect("a", 0, 0, 10, 5);
        let b = make_rect("b", 15, 0, 10, 5);

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);
        candidates.insert(b.0.clone(), b.1);

        // From a, Right → b
        let result = spatial_nearest(&a.1, SpatialDirection::Right, &candidates, "a");
        assert_eq!(result.as_deref(), Some("b"));
    }

    #[test]
    fn nearest_left_returns_node_in_previous_column() {
        let a = make_rect("a", 0, 0, 10, 5);
        let b = make_rect("b", 15, 0, 10, 5);

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);
        candidates.insert(b.0.clone(), b.1);

        // From b, Left → a
        let result = spatial_nearest(&b.1, SpatialDirection::Left, &candidates, "b");
        assert_eq!(result.as_deref(), Some("a"));
    }

    #[test]
    fn nearest_returns_none_at_boundary() {
        let a = make_rect("a", 0, 0, 10, 5);

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);

        // From a, Right → nothing
        assert!(spatial_nearest(&a.1, SpatialDirection::Right, &candidates, "a").is_none());
        // From a, Down → nothing
        assert!(spatial_nearest(&a.1, SpatialDirection::Down, &candidates, "a").is_none());
        // From a, Left → nothing
        assert!(spatial_nearest(&a.1, SpatialDirection::Left, &candidates, "a").is_none());
        // From a, Up → nothing
        assert!(spatial_nearest(&a.1, SpatialDirection::Up, &candidates, "a").is_none());
    }

    #[test]
    fn nearest_prefers_overlapping_cross_axis() {
        // Diamond layout: a at top, b and c in middle column, d at bottom.
        // a is at x=5..15, b at x=0..10, c at x=20..30.
        let a = make_rect("a", 5, 0, 10, 5);
        let b = make_rect("b", 0, 10, 10, 5);
        let c = make_rect("c", 20, 10, 10, 5);

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);
        candidates.insert(b.0.clone(), b.1);
        candidates.insert(c.0.clone(), c.1);

        // From a, Down: both b and c are below. b overlaps X with a, c does not.
        let result = spatial_nearest(&a.1, SpatialDirection::Down, &candidates, "a");
        assert_eq!(result.as_deref(), Some("b"));
    }

    #[test]
    fn nearest_uses_diagonal_fallback() {
        // a at top-left, b at bottom-right (no X overlap).
        let a = make_rect("a", 0, 0, 10, 5);
        let b = make_rect("b", 20, 10, 10, 5);

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);
        candidates.insert(b.0.clone(), b.1);

        // From a, Down: b doesn't overlap X, but delta_y=10 >= delta_x=15? No.
        // b center_x=25, a center_x=5, delta_x=20. delta_y: b center_y=12, a center_y=2, delta_y=10.
        // 10 < 20, so angle constraint fails → None.
        let result = spatial_nearest(&a.1, SpatialDirection::Down, &candidates, "a");
        assert!(result.is_none());
    }

    #[test]
    fn nearest_diagonal_fallback_accepts_reasonable_angle() {
        // a at top, b at bottom-right with shallow angle.
        let a = make_rect("a", 0, 0, 10, 5); // center: (5, 2)
        let b = make_rect("b", 8, 10, 10, 5); // center: (13, 12)

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);
        candidates.insert(b.0.clone(), b.1);

        // From a, Down: b doesn't overlap X.
        // delta_y = 12-2 = 10, delta_x = |13-5| = 8.
        // 10 >= 8 → angle constraint passes.
        let result = spatial_nearest(&a.1, SpatialDirection::Down, &candidates, "a");
        assert_eq!(result.as_deref(), Some("b"));
    }

    #[test]
    fn nearest_skips_self() {
        let a = make_rect("a", 0, 0, 10, 5);
        let b = make_rect("b", 0, 6, 10, 5);

        let mut candidates = HashMap::new();
        candidates.insert(a.0.clone(), a.1);
        candidates.insert(b.0.clone(), b.1);

        // From a, Down should find b, not a itself.
        let result = spatial_nearest(&a.1, SpatialDirection::Down, &candidates, "a");
        assert_eq!(result.as_deref(), Some("b"));
    }
}
