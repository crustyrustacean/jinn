//! Connection routing between node ports.
//!
//! Provides the [`ConnectionRouter`] trait and [`SimpleRouter`] which produces
//! L-shaped paths (right → vertical → right) between output and input ports.

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

use crate::port::port_type_color;
use nullslop_workflow::port::PortType;

/// A cell on a connection path, with its direction information for character selection.
#[derive(Debug, Clone, Copy)]
pub struct PathCell {
    /// Absolute position (i32 allows negative coords for off-screen cells).
    pub pos: (i32, i32),
    /// Box-drawing character to render at this position.
    pub char: char,
}

/// A connection router produces a path of cells between two port positions.
pub trait ConnectionRouter {
    /// Route a connection from an output port to an input port.
    ///
    /// Returns a list of [`PathCell`]s forming the visual path.
    fn route(from: (i32, i32), to: (i32, i32), node_rects: &[Rect]) -> Vec<PathCell>;
}

/// Simple L-shaped router: moves horizontally right to the midpoint, then vertically,
/// then horizontally right to the target.
pub struct SimpleRouter;

impl ConnectionRouter for SimpleRouter {
    fn route(from: (i32, i32), to: (i32, i32), _node_rects: &[Rect]) -> Vec<PathCell> {
        let (x1, y1) = from;
        let (x2, y2) = to;

        let mut cells: Vec<(i32, i32)> = Vec::new();

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
        #[expect(
            clippy::indexing_slicing,
            reason = "indices are bounds-checked by enumerate"
        )]
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

/// Selects the correct box-drawing character for a cell given the set of
/// directions in which wires extend from it.
///
/// Handles straight lines (2 dirs), corners (2 dirs), tee junctions (3 dirs),
/// crosses (4 dirs), and end caps (1 dir).
fn box_char_from_dirs(dirs: &HashSet<Dir2D>) -> char {
    let has_up = dirs.contains(&Dir2D::Up);
    let has_down = dirs.contains(&Dir2D::Down);
    let has_left = dirs.contains(&Dir2D::Left);
    let has_right = dirs.contains(&Dir2D::Right);

    match (has_up, has_down, has_left, has_right) {
        // Straight lines
        (false, false, true, true) => '─',
        (true, true, false, false) => '│',
        // End caps (single direction)
        (false, false, true, false) | (false, false, false, true) => '─',
        (true, false, false, false) | (false, true, false, false) => '│',
        // Corners
        (false, true, false, true) => '╭',
        (false, true, true, false) => '╮',
        (true, false, false, true) => '╰',
        (true, false, true, false) => '╯',
        // Tee junctions
        (false, true, true, true) => '┬',
        (true, false, true, true) => '┴',
        (true, true, false, true) => '├',
        (true, true, true, false) => '┤',
        // Cross
        (true, true, true, true) => '┼',
        // Fallback (should not happen with valid dirs)
        _ => '┼',
    }
}

/// Given the previous, current, and next cell positions, pick the correct
/// box-drawing character.
fn box_char(prev: Option<(i32, i32)>, curr: (i32, i32), next: Option<(i32, i32)>) -> char {
    let mut dirs = HashSet::new();
    if let Some(d) = prev.and_then(|p| dir_toward(curr, p)) {
        dirs.insert(d);
    }
    if let Some(d) = next.and_then(|n| dir_toward(curr, n)) {
        dirs.insert(d);
    }
    box_char_from_dirs(&dirs)
}

/// Direction from one cell toward an adjacent cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Dir2D {
    Up,
    Down,
    Left,
    Right,
}

/// Computes the direction from `curr` toward `other`.
///
/// Returns `None` if the cells are not orthogonally adjacent.
pub(crate) fn dir_toward(curr: (i32, i32), other: (i32, i32)) -> Option<Dir2D> {
    let dx = other.0 - curr.0;
    let dy = other.1 - curr.1;
    match (dx.signum(), dy.signum()) {
        (1, 0) => Some(Dir2D::Right),
        (-1, 0) => Some(Dir2D::Left),
        (0, 1) => Some(Dir2D::Down),
        (0, -1) => Some(Dir2D::Up),
        _ => None,
    }
}

/// Renders a connection path into a ratatui buffer with the given port type's color.
///
/// Skips cells with negative coordinates or outside the buffer area.
pub fn render_path(buf: &mut Buffer, path: &[PathCell], port_type: PortType, area: Rect) {
    let color = port_type_color(port_type);
    for cell in path {
        let (x, y) = cell.pos;
        // Skip cells outside the buffer area or with negative coordinates.
        if x < 0 || y < 0 {
            continue;
        }
        let Ok(x) = u16::try_from(x) else {
            continue;
        };
        let Ok(y) = u16::try_from(y) else {
            continue;
        };
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
#[expect(clippy::indexing_slicing, reason = "test indices are known-valid")]
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
    fn turn_source_above_target_uses_correct_corners() {
        // Wire goes right then down (y1=0 < y2=5).
        let path = SimpleRouter::route((0, 0), (10, 5), &[]);
        let turns: Vec<_> = path
            .iter()
            .filter(|c| matches!(c.char, '╮' | '╭' | '╯' | '╰'))
            .collect();
        assert_eq!(turns.len(), 2, "should have exactly two turns");
        // First turn at (5,0): arms go LEFT and DOWN = ╮
        assert_eq!(
            turns[0].char, '╮',
            "first turn should be ╮ (left+down arms) at {:?}",
            turns[0].pos
        );
        // Second turn at (5,5): arms go UP and RIGHT = ╰
        assert_eq!(
            turns[1].char, '╰',
            "second turn should be ╰ (up+right arms) at {:?}",
            turns[1].pos
        );
    }

    #[test]
    fn turn_source_below_target_uses_correct_corners() {
        // Wire goes right then up (y1=5 > y2=0).
        let path = SimpleRouter::route((0, 5), (10, 0), &[]);
        let turns: Vec<_> = path
            .iter()
            .filter(|c| matches!(c.char, '╮' | '╭' | '╯' | '╰'))
            .collect();
        assert_eq!(turns.len(), 2, "should have exactly two turns");
        // First turn at (5,5): arms go LEFT and UP = ╯
        assert_eq!(
            turns[0].char, '╯',
            "first turn should be ╯ (left+up arms) at {:?}",
            turns[0].pos
        );
        // Second turn at (5,0): arms go DOWN and RIGHT = ╭
        assert_eq!(
            turns[1].char, '╭',
            "second turn should be ╭ (down+right arms) at {:?}",
            turns[1].pos
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

    // --- box_char_from_dirs tests ---

    fn dirs(set: &[Dir2D]) -> HashSet<Dir2D> {
        set.iter().copied().collect()
    }

    #[test]
    fn dirs_straight_horizontal() {
        assert_eq!(box_char_from_dirs(&dirs(&[Dir2D::Left, Dir2D::Right])), '─');
    }

    #[test]
    fn dirs_straight_vertical() {
        assert_eq!(box_char_from_dirs(&dirs(&[Dir2D::Up, Dir2D::Down])), '│');
    }

    #[test]
    fn dirs_corner_top_left() {
        assert_eq!(box_char_from_dirs(&dirs(&[Dir2D::Right, Dir2D::Down])), '╭');
    }

    #[test]
    fn dirs_corner_top_right() {
        assert_eq!(box_char_from_dirs(&dirs(&[Dir2D::Left, Dir2D::Down])), '╮');
    }

    #[test]
    fn dirs_corner_bottom_left() {
        assert_eq!(box_char_from_dirs(&dirs(&[Dir2D::Right, Dir2D::Up])), '╰');
    }

    #[test]
    fn dirs_corner_bottom_right() {
        assert_eq!(box_char_from_dirs(&dirs(&[Dir2D::Left, Dir2D::Up])), '╯');
    }

    #[test]
    fn dirs_tee_down() {
        assert_eq!(
            box_char_from_dirs(&dirs(&[Dir2D::Left, Dir2D::Right, Dir2D::Down])),
            '┬'
        );
    }

    #[test]
    fn dirs_tee_up() {
        assert_eq!(
            box_char_from_dirs(&dirs(&[Dir2D::Left, Dir2D::Right, Dir2D::Up])),
            '┴'
        );
    }

    #[test]
    fn dirs_tee_right() {
        assert_eq!(
            box_char_from_dirs(&dirs(&[Dir2D::Up, Dir2D::Down, Dir2D::Right])),
            '├'
        );
    }

    #[test]
    fn dirs_tee_left() {
        assert_eq!(
            box_char_from_dirs(&dirs(&[Dir2D::Up, Dir2D::Down, Dir2D::Left])),
            '┤'
        );
    }

    #[test]
    fn dirs_cross() {
        assert_eq!(
            box_char_from_dirs(&dirs(&[
                Dir2D::Up,
                Dir2D::Down,
                Dir2D::Left,
                Dir2D::Right
            ])),
            '┼'
        );
    }

    #[test]
    fn dirs_end_cap_right() {
        assert_eq!(box_char_from_dirs(&dirs(&[Dir2D::Right])), '─');
    }

    #[test]
    fn dirs_end_cap_down() {
        assert_eq!(box_char_from_dirs(&dirs(&[Dir2D::Down])), '│');
    }
}
