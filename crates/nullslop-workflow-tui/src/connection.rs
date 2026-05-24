//! Connection routing between node ports.
//!
//! Provides the [`ConnectionRouter`] trait and [`SimpleRouter`] which produces
//! L-shaped paths (right → vertical → right) between output and input ports.

use std::collections::{HashMap, HashSet};

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Color;

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

/// Merged direction and color information for a cell shared by multiple paths.
pub struct CellInfo {
    /// All directions in which wires extend from this cell.
    pub dirs: HashSet<Dir2D>,
    /// The port type of the first path to contribute to this cell.
    pub port_type: PortType,
    /// True if multiple different port types contribute to this cell.
    pub mixed: bool,
}

/// Inserts a routed path into a merged grid, accumulating direction information.
///
/// For each cell in the path, computes the directions toward its neighbors
/// and merges them into the grid entry for that position. Tracks port type
/// mixing for color decisions.
pub(crate) fn insert_path_into_grid(
    grid: &mut HashMap<(i32, i32), CellInfo>,
    path: &[PathCell],
    port_type: PortType,
) {
    #[expect(
        clippy::indexing_slicing,
        reason = "indices are bounds-checked by enumerate"
    )]
    for (i, cell) in path.iter().enumerate() {
        let entry = grid.entry(cell.pos).or_insert_with(|| CellInfo {
            dirs: HashSet::new(),
            port_type,
            mixed: false,
        });

        if entry.port_type != port_type {
            entry.mixed = true;
        }

        if i > 0
            && let Some(d) = dir_toward(cell.pos, path[i - 1].pos) {
                entry.dirs.insert(d);
            }
        if i + 1 < path.len()
            && let Some(d) = dir_toward(cell.pos, path[i + 1].pos) {
                entry.dirs.insert(d);
            }
    }
}

/// Renders a merged path grid into a ratatui buffer.
///
/// Cells with 3+ directions (junctions) or mixed port types render in gray.
/// Other cells render in their port type's color.
pub fn render_merged_grid(buf: &mut Buffer, grid: &HashMap<(i32, i32), CellInfo>, area: Rect) {
    for (pos, info) in grid {
        let (x, y) = *pos;
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

        let ch = box_char_from_dirs(&info.dirs);
        let is_junction = info.dirs.len() >= 3;
        let color = if is_junction || info.mixed {
            Color::DarkGray
        } else {
            port_type_color(info.port_type)
        };

        let p = Position::new(x, y);
        if let Some(cell_buf) = buf.cell_mut(p) {
            cell_buf.set_char(ch);
            cell_buf.fg = color;
        }
    }
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
        let mid_x = i32::midpoint(x1, x2);

        // Horizontal right from source to midpoint.
        for x in x1..=mid_x {
            cells.push((x, y1));
        }

        // Vertical from y1 to y2 at mid_x.
        match y1.cmp(&y2) {
            std::cmp::Ordering::Less => {
                for y in (y1 + 1)..=y2 {
                    cells.push((mid_x, y));
                }
            }
            std::cmp::Ordering::Greater => {
                for y in (y2..y1).rev() {
                    cells.push((mid_x, y));
                }
            }
            std::cmp::Ordering::Equal => {
                // y1 == y2: no vertical segment needed.
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
                let prev = (i > 0).then(|| cells[i - 1]);
                let next = (i + 1 < cells.len()).then(|| cells[i + 1]);
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

    #[expect(clippy::match_same_arms, reason = "end caps intentionally match their parent line style")]
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
pub enum Dir2D {
    /// Up direction.
    Up,
    /// Down direction.
    Down,
    /// Left direction.
    Left,
    /// Right direction.
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
            cell_buf.fg = color;
        }
    }
}

#[cfg(test)]
#[expect(clippy::indexing_slicing, reason = "test indices are known-valid")]
#[expect(clippy::expect_used, reason = "test assertions")]
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
        let mid_x = i32::midpoint(10, 20); // 15
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
            box_char_from_dirs(&dirs(&[Dir2D::Up, Dir2D::Down, Dir2D::Left, Dir2D::Right])),
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

    // --- Grid merge tests ---

    #[test]
    fn grid_fan_out_produces_tee() {
        let mut grid: HashMap<(i32, i32), CellInfo> = HashMap::new();
        // Two paths from same source to different targets.
        // Route 1: (0,0)→(10,2), Route 2: (0,0)→(10,5).
        // Both share horizontal trunk then diverge at midpoint (5,0)→(5,2).
        // At (5,2): path1 turns right, path2 continues down → dirs={Up,Right,Down} = ├
        let path1 = SimpleRouter::route((0, 0), (10, 2), &[]);
        let path2 = SimpleRouter::route((0, 0), (10, 5), &[]);
        insert_path_into_grid(&mut grid, &path1, PortType::String);
        insert_path_into_grid(&mut grid, &path2, PortType::String);

        // Find a junction cell (3+ dirs).
        let junctions: Vec<_> = grid
            .iter()
            .filter(|(_, info)| info.dirs.len() >= 3)
            .collect();
        assert!(!junctions.is_empty(), "should have at least one junction");

        // The junction should be some kind of tee.
        let has_tee = junctions.iter().any(|(_, info)| {
            let ch = box_char_from_dirs(&info.dirs);
            matches!(ch, '┬' | '┴' | '├' | '┤' | '┼')
        });
        assert!(
            has_tee,
            "fan-out should produce a tee junction, got {:?}",
            junctions
                .iter()
                .map(|(_, info)| box_char_from_dirs(&info.dirs))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn grid_fan_in_produces_tee() {
        let mut grid: HashMap<(i32, i32), CellInfo> = HashMap::new();
        // Two paths converging on the same target.
        // Route 1: (0,0)→(10,3), Route 2: (0,5)→(10,3).
        // Both meet at midpoint column 5, then share the final horizontal run.
        // At (5,3): path1 comes from above, path2 from below → dirs={Up,Down,Right} = ├
        let path1 = SimpleRouter::route((0, 0), (10, 3), &[]);
        let path2 = SimpleRouter::route((0, 5), (10, 3), &[]);
        insert_path_into_grid(&mut grid, &path1, PortType::String);
        insert_path_into_grid(&mut grid, &path2, PortType::String);

        let junctions: Vec<_> = grid
            .iter()
            .filter(|(_, info)| info.dirs.len() >= 3)
            .collect();
        assert!(!junctions.is_empty(), "should have at least one junction");

        let has_tee = junctions.iter().any(|(_, info)| {
            let ch = box_char_from_dirs(&info.dirs);
            matches!(ch, '┬' | '┴' | '├' | '┤' | '┼')
        });
        assert!(has_tee, "fan-in should produce a tee junction");
    }

    #[test]
    fn grid_junction_renders_gray() {
        let mut grid: HashMap<(i32, i32), CellInfo> = HashMap::new();
        let path1 = SimpleRouter::route((0, 0), (10, 2), &[]);
        let path2 = SimpleRouter::route((0, 0), (10, 5), &[]);
        insert_path_into_grid(&mut grid, &path1, PortType::String);
        insert_path_into_grid(&mut grid, &path2, PortType::String);

        let junction = grid
            .iter()
            .find(|(_, info)| info.dirs.len() >= 3)
            .expect("should have a junction");

        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let mut buf = Buffer::empty(area);
        render_merged_grid(&mut buf, &grid, area);

        let (pos, _) = junction;
        let cell = buf
            .cell(Position::new(
                u16::try_from(pos.0).unwrap(),
                u16::try_from(pos.1).unwrap(),
            ))
            .unwrap();
        assert_eq!(cell.fg, Color::DarkGray);
    }

    #[test]
    fn grid_non_junction_same_type_keeps_color() {
        let mut grid: HashMap<(i32, i32), CellInfo> = HashMap::new();
        let path1 = SimpleRouter::route((0, 0), (10, 2), &[]);
        let path2 = SimpleRouter::route((0, 0), (10, 5), &[]);
        insert_path_into_grid(&mut grid, &path1, PortType::String);
        insert_path_into_grid(&mut grid, &path2, PortType::String);

        // Find a non-junction cell on the shared trunk (2 dirs, same type).
        let non_junction = grid
            .iter()
            .find(|(_, info)| info.dirs.len() == 2 && !info.mixed)
            .expect("should have a non-junction cell");

        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let mut buf = Buffer::empty(area);
        render_merged_grid(&mut buf, &grid, area);

        let (pos, _) = non_junction;
        let cell = buf
            .cell(Position::new(
                u16::try_from(pos.0).unwrap(),
                u16::try_from(pos.1).unwrap(),
            ))
            .unwrap();
        assert_eq!(
            cell.fg,
            Color::Green,
            "non-junction same-type cell should be green"
        );
    }

    #[test]
    fn grid_mixed_type_cell_is_marked_mixed() {
        let mut grid: HashMap<(i32, i32), CellInfo> = HashMap::new();
        // Two overlapping horizontal paths, different port types.
        let path1 = SimpleRouter::route((0, 0), (10, 0), &[]);
        let path2 = SimpleRouter::route((0, 0), (10, 0), &[]);
        insert_path_into_grid(&mut grid, &path1, PortType::String);
        insert_path_into_grid(&mut grid, &path2, PortType::Json);

        let cell = grid
            .get(&(5, 0))
            .expect("should have cell at shared position");
        assert!(cell.mixed, "shared cell should be marked mixed");
    }
}
