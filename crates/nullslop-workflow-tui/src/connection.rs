//! Connection routing between node ports.
//!
//! Provides the [`ConnectionRouter`] trait and [`SimpleRouter`] which produces
//! L-shaped paths (right → vertical → right) between output and input ports.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::Color,
};

use crate::port::port_type_color;
use nullslop_workflow::port::PortType;

/// A cell on a connection path, with its direction information for character selection.
#[derive(Debug, Clone, Copy)]
pub struct PathCell {
    /// Absolute position.
    pub pos: (u16, u16),
    /// Box-drawing character to render at this position.
    pub char: char,
}

/// A connection router produces a path of cells between two port positions.
pub trait ConnectionRouter {
    /// Route a connection from an output port to an input port.
    ///
    /// Returns a list of [`PathCell`]s forming the visual path.
    fn route(from: (u16, u16), to: (u16, u16), node_rects: &[Rect]) -> Vec<PathCell>;
}

/// Simple L-shaped router: moves horizontally right to the midpoint, then vertically,
/// then horizontally right to the target.
pub struct SimpleRouter;

impl ConnectionRouter for SimpleRouter {
    fn route(from: (u16, u16), to: (u16, u16), _node_rects: &[Rect]) -> Vec<PathCell> {
        let (x1, y1) = from;
        let (x2, y2) = to;

        let mut cells: Vec<(u16, u16)> = Vec::new();

        // Midpoint x: halfway between source and target.
        let mid_x = (x1 + x2) / 2;

        // Horizontal right from source to midpoint.
        for x in x1..=mid_x {
            cells.push((x, y1));
        }

        // Vertical from y1 to y2 at mid_x.
        if y1 < y2 {
            for y in (y1 + 1)..=y2 {
                cells.push((mid_x, y));
            }
        } else if y1 > y2 {
            for y in (y2..y1).rev() {
                cells.push((mid_x, y));
            }
        }

        // Horizontal right from midpoint to target.
        for x in (mid_x + 1)..=x2 {
            cells.push((x, y2));
        }

        // Deduplicate (mid_x may appear twice at the junction).
        cells.dedup();

        // Convert to PathCells with box-drawing characters.
        #[expect(clippy::indexing_slicing, reason = "indices are bounds-checked by enumerate")]
        let path_cells: Vec<PathCell> = cells
            .iter()
            .enumerate()
            .map(|(i, &pos)| {
                let prev = if i > 0 { Some(cells[i - 1]) } else { None };
                let next = if i + 1 < cells.len() {
                    Some(cells[i + 1])
                } else {
                    None
                };
                PathCell {
                    pos,
                    char: box_char(prev, pos, next),
                }
            })
            .collect();

        path_cells
    }
}

/// Given the previous, current, and next cell positions, pick the correct
/// box-drawing character.
fn box_char(prev: Option<(u16, u16)>, curr: (u16, u16), next: Option<(u16, u16)>) -> char {
    let from_dir = prev.and_then(|p| direction(p, curr));
    let to_dir = next.and_then(|n| direction(curr, n));

    match (from_dir, to_dir) {
        // Straight lines
        (Some(Dir::H), Some(Dir::H)) => '─',
        (Some(Dir::V), Some(Dir::V)) => '│',
        // Start/end caps (only one direction)
        (None, Some(Dir::H)) | (Some(Dir::H), None) => '─',
        (None, Some(Dir::V)) | (Some(Dir::V), None) => '│',
        // Turns
        (Some(Dir::H), Some(Dir::V)) | (Some(Dir::V), Some(Dir::H)) => {
            // Need to know which quadrant to pick the right corner.
            turn_char(prev, curr, next)
        }
        // Fallback
        _ => '┼',
    }
}

/// Direction between two adjacent cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    H, // horizontal
    V, // vertical
}

fn direction(a: (u16, u16), b: (u16, u16)) -> Option<Dir> {
    match a {
        (ax, ay) if ax == b.0 && ay != b.1 => Some(Dir::V),
        (ax, ay) if ax != b.0 && ay == b.1 => Some(Dir::H),
        _ => None,
    }
}

/// Pick the correct rounded corner character for a turn.
fn turn_char(prev: Option<(u16, u16)>, curr: (u16, u16), next: Option<(u16, u16)>) -> char {
    let (cx, cy) = curr;
    let (px, py) = prev.unwrap_or(curr);
    let (nx, ny) = next.unwrap_or(curr);

    // Determine the two segments meeting at curr.
    let from_left = px < cx;
    let from_right = px > cx;
    let from_above = py < cy;
    let from_below = py > cy;
    let to_left = nx < cx;
    let to_right = nx > cx;
    let to_above = ny < cy;
    let to_below = ny > cy;

    // ╭ top-left corner: comes from above or right, goes right or below
    if (from_above || to_below) && (from_right || to_right) && !(from_left || to_left) {
        return '╭';
    }
    // ╮ top-right corner: comes from above or left, goes left or below
    if (from_above || to_below) && (from_left || to_left) && !(from_right || to_right) {
        return '╮';
    }
    // ╰ bottom-left corner: comes from below or right, goes right or above
    if (from_below || to_above) && (from_right || to_right) && !(from_left || to_left) {
        return '╰';
    }
    // ╯ bottom-right corner: comes from below or left, goes left or above
    if (from_below || to_above) && (from_left || to_left) && !(from_right || to_right) {
        return '╯';
    }

    '┼' // fallback
}

/// Renders a connection path into a ratatui buffer with the given port type's color.
pub fn render_path(buf: &mut Buffer, path: &[PathCell], port_type: PortType, area: Rect) {
    let color = port_type_color(port_type);
    for cell in path {
        let (x, y) = cell.pos;
        // Skip cells outside the buffer area.
        if x >= area.x + area.width || y >= area.y + area.height {
            continue;
        }
        let pos = Position::new(x, y);
        if let Some(cell_buf) = buf.cell_mut(pos) {
            cell_buf.set_char(cell.char);
            cell_buf.fg = color.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_shaped_path_connects_side_by_side() {
        // Source output at (10, 2), target input at (20, 5).
        let path = SimpleRouter::route((10, 2), (20, 5), &[]);

        // Path should start at (10, 2) and end at (20, 5).
        assert_eq!(path.first().unwrap().pos, (10, 2));
        assert_eq!(path.last().unwrap().pos, (20, 5));

        // Should contain the midpoint vertical segment.
        let mid_x = (10 + 20) / 2; // 15
        assert!(path.iter().any(|c| c.pos == (mid_x, 3)));
        assert!(path.iter().any(|c| c.pos == (mid_x, 4)));
    }

    #[test]
    fn horizontal_path_uses_dash() {
        let path = SimpleRouter::route((0, 0), (5, 0), &[]);
        // All interior cells should be '─'.
        for cell in &path {
            assert_eq!(cell.char, '─', "at {:?}", cell.pos);
        }
    }

    #[test]
    fn vertical_path_uses_pipe() {
        let path = SimpleRouter::route((3, 0), (3, 5), &[]);
        // All interior cells should be '│'.
        for cell in &path {
            assert_eq!(cell.char, '│', "at {:?}", cell.pos);
        }
    }

    #[test]
    fn turn_uses_corner_characters() {
        let path = SimpleRouter::route((0, 0), (10, 5), &[]);

        // Find the turn point (mid_x, y1) → (mid_x, y2) transition.
        let has_turn = path.iter().any(|c| matches!(c.char, '╭' | '╮' | '╰' | '╯'));
        assert!(
            has_turn,
            "path should contain at least one corner character"
        );
    }

    #[test]
    fn connection_color_matches_port_type() {
        let string_color = port_type_color(PortType::String);
        let json_color = port_type_color(PortType::Json);
        assert_ne!(
            string_color, json_color,
            "String and Json should have different colors"
        );
    }
}
