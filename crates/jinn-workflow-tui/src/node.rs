//! Visual node rendering.
//!
//! A [`VisualNode`] computes its own dimensions from port definitions and
//! renders as a rounded box with typed ports on the border edges, a status
//! indicator, and a title bar.

use jinn_workflow::engine::NodeStatus;
use jinn_workflow::port::{PortDef, PortType};
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;

use jinn_workflow::spatial_layout;

use crate::port::{port_type_color, port_type_label};
use crate::status::{status_color, status_symbol};

/// Which side of the node box a port sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortSide {
    /// Input ports sit on the left border.
    Left,
    /// Output ports sit on the right border.
    Right,
}

/// A port rendered on a node's border.
#[derive(Debug, Clone)]
pub struct VisualPort {
    /// Port name.
    pub name: String,
    /// Port type (determines indicator color).
    pub port_type: PortType,
    /// Which side of the node this port sits on.
    pub side: PortSide,
    /// Row offset from the top of the node box (0-indexed, row 0 = top border).
    pub row_offset: u16,
}

/// A node rendered as a rounded box with typed ports.
///
/// Call [`compute`](Self::compute) to create one from port definitions and
/// a status, then call [`render`](Self::render) to draw it into a buffer.
#[derive(Debug, Clone)]
pub struct VisualNode {
    /// Node name (displayed in the title bar).
    pub name: String,
    /// Top-left corner column position.
    pub x: u16,
    /// Top-left corner row position.
    pub y: u16,
    /// Box width in cells (including borders).
    pub width: u16,
    /// Box height in cells (including borders).
    pub height: u16,
    /// Input ports (rendered on the left border, top-aligned).
    pub input_ports: Vec<VisualPort>,
    /// Output ports (rendered on the right border, bottom-aligned).
    pub output_ports: Vec<VisualPort>,
    /// Execution status of this node.
    pub status: NodeStatus,
}

/// Horizontal padding inside the border (space between border and content).
const H_PAD: usize = 1;

impl VisualNode {
    /// Computes a `VisualNode` from its name, port definitions, and status.
    ///
    /// The node is placed at `(0, 0)` - the caller assigns the final position
    /// via `x` and `y` fields.
    #[must_use]
    pub fn compute(
        name: String,
        input_defs: &[PortDef],
        output_defs: &[PortDef],
        status: NodeStatus,
    ) -> Self {
        let content_width = Self::compute_content_width(&name, input_defs, output_defs);
        let width = u16::try_from(content_width + 2 + 2 * H_PAD).unwrap_or(u16::MAX);

        let input_count = input_defs.len().max(1);
        let output_count = output_defs.len().max(1);
        // height = top_border + inputs + gap + outputs + bottom_border
        let height = u16::try_from(1 + input_count + 1 + output_count + 1).unwrap_or(u16::MAX);

        // Assign row offsets for input ports (start at row 1, just below top border).
        let input_ports: Vec<VisualPort> = input_defs
            .iter()
            .enumerate()
            .map(|(i, def)| VisualPort {
                name: def.name.clone(),
                port_type: def.value_type,
                side: PortSide::Left,
                row_offset: u16::try_from(1 + i).unwrap_or(u16::MAX),
            })
            .collect();

        // Assign row offsets for output ports (start from bottom - 1, going up).
        let output_ports: Vec<VisualPort> = output_defs
            .iter()
            .enumerate()
            .map(|(i, def)| VisualPort {
                name: def.name.clone(),
                port_type: def.value_type,
                side: PortSide::Right,
                // Bottom border is at row (height-1), so last output row is (height-2).
                row_offset: height - 2 - u16::try_from(i).unwrap_or(u16::MAX),
            })
            .collect();

        Self {
            name,
            x: 0,
            y: 0,
            width,
            height,
            input_ports,
            output_ports,
            status,
        }
    }

    /// Computes the content width needed for the node box.
    ///
    /// Width is the maximum of:
    /// - Title length + space for status indicator
    /// - Longest "Type name" label among all ports
    ///
    /// Delegates to [`spatial_layout::compute_node_size`] for the math.
    fn compute_content_width(name: &str, inputs: &[PortDef], outputs: &[PortDef]) -> usize {
        let (w, _h) = spatial_layout::compute_node_size(name, inputs, outputs);
        // Subtract borders + padding to get content width.
        usize::from(w).saturating_sub(2 + 2 * H_PAD)
    }

    /// Renders the node into the given buffer at the node's `(x, y)` position.
    ///
    /// If `selected` is true, the border is drawn with a highlight color.
    /// The `tick` counter drives the spinner animation for running nodes.
    /// If the node has `AwaitingInput` status, the border uses `awaiting_input_color`
    /// instead of the default selected/unselected color.
    pub fn render(&self, buf: &mut Buffer, selected: bool, tick: u8, awaiting_input_color: Color) {
        let border_color = if selected {
            Color::White
        } else if self.status == NodeStatus::AwaitingInput {
            awaiting_input_color
        } else {
            Color::DarkGray
        };
        let border_style = Style::default().fg(border_color);

        self.render_borders(buf, border_style);
        self.render_title(buf, border_style, tick);
        self.render_port_labels(buf);
        self.render_port_indicators(buf);
        self.render_status(buf, tick, awaiting_input_color);
    }

    /// Renders the rounded box borders.
    fn render_borders(&self, buf: &mut Buffer, style: Style) {
        let width = usize::from(self.width);
        let height = usize::from(self.height);

        // Top border: ╭─...─╮
        self.set_cell(buf, 0, 0, "╭", style);
        for col in 1..width.saturating_sub(1) {
            self.set_cell(buf, u16::try_from(col).unwrap_or(u16::MAX), 0, "─", style);
        }
        self.set_cell(buf, self.width.saturating_sub(1), 0, "╮", style);

        // Side borders: │ ... │
        for row in 1..height.saturating_sub(1) {
            self.set_cell(buf, 0, u16::try_from(row).unwrap_or(u16::MAX), "│", style);
            self.set_cell(
                buf,
                self.width.saturating_sub(1),
                u16::try_from(row).unwrap_or(u16::MAX),
                "│",
                style,
            );
        }

        // Bottom border: ╰─...─╯
        let last_row = self.height.saturating_sub(1);
        self.set_cell(buf, 0, last_row, "╰", style);
        for col in 1..width.saturating_sub(1) {
            self.set_cell(
                buf,
                u16::try_from(col).unwrap_or(u16::MAX),
                last_row,
                "─",
                style,
            );
        }
        self.set_cell(buf, self.width.saturating_sub(1), last_row, "╯", style);
    }

    /// Renders the title bar with node name.
    fn render_title(&self, buf: &mut Buffer, border_style: Style, _tick: u8) {
        // Title goes in the top border after the first two chars "╭─"
        let start_col = 2;
        let max_title_len = usize::from(self.width).saturating_sub(4); // ╭─  ╮
        let title = truncate_str(&self.name, max_title_len);
        for (i, g) in title.graphemes(true).enumerate() {
            let col = start_col + u16::try_from(i).unwrap_or(u16::MAX);
            if col < self.width.saturating_sub(2) {
                self.set_cell(buf, col, 0, g, border_style);
            }
        }
    }

    /// Renders the status indicator (circle/spinner) at the top-right area.
    fn render_status(&self, buf: &mut Buffer, tick: u8, awaiting_input_color: Color) {
        let symbol = status_symbol(self.status, tick);
        let color = status_color(self.status, awaiting_input_color);
        // Place status indicator at column (width - 2), row 0 (top border).
        let col = self.width.saturating_sub(2);
        self.set_cell(buf, col, 0, symbol, Style::default().fg(color));
    }

    /// Renders the type+name labels for input and output ports inside the box.
    fn render_port_labels(&self, buf: &mut Buffer) {
        let content_start = 1 + H_PAD; // after left border + padding
        let content_width = usize::from(self.width).saturating_sub(2 + 2 * H_PAD);

        // Input labels (left-aligned).
        for port in &self.input_ports {
            let label = format!("{} {}", port_type_label(port.port_type), port.name);
            let label = truncate_str(&label, content_width);
            let style = Style::default().fg(port_type_color(port.port_type));
            self.render_text(buf, content_start, port.row_offset, &label, style);
        }

        // Output labels (right-aligned).
        for port in &self.output_ports {
            let label = format!("{} {}", port_type_label(port.port_type), port.name);
            let label = truncate_str(&label, content_width);
            let style = Style::default().fg(port_type_color(port.port_type));
            // Right-align: compute starting column.
            let label_len = label.graphemes(true).count();
            let start_col = (content_start + content_width)
                .saturating_sub(label_len)
                .max(content_start);
            self.render_text(buf, start_col, port.row_offset, &label, style);
        }
    }

    /// Renders port indicators (○/●) on the left and right borders.
    fn render_port_indicators(&self, buf: &mut Buffer) {
        let style = Style::default().add_modifier(Modifier::BOLD);

        // Input ports on left border (column 0).
        for port in &self.input_ports {
            let color = port_type_color(port.port_type);
            self.set_cell(buf, 0, port.row_offset, "○", style.fg(color));
        }

        // Output ports on right border (column width - 1).
        for port in &self.output_ports {
            let color = port_type_color(port.port_type);
            self.set_cell(
                buf,
                self.width.saturating_sub(1),
                port.row_offset,
                "○",
                style.fg(color),
            );
        }
    }

    /// Sets a single cell in the buffer at local coordinates (relative to `x, y`).
    fn set_cell(
        &self,
        buf: &mut Buffer,
        local_col: u16,
        local_row: u16,
        symbol: &str,
        style: Style,
    ) {
        let col = self.x + local_col;
        let row = self.y + local_row;
        if let Some(cell) = buf.cell_mut(Position::new(col, row)) {
            cell.set_symbol(symbol);
            cell.set_style(style);
        }
    }

    /// Renders text starting at local coordinates.
    fn render_text(&self, buf: &mut Buffer, start_col: usize, row: u16, text: &str, style: Style) {
        for (i, g) in text.graphemes(true).enumerate() {
            let col = u16::try_from(start_col + i).unwrap_or(u16::MAX);
            if col < self.width.saturating_sub(1) {
                self.set_cell(buf, col, row, g, style);
            }
        }
    }

    /// Returns the bounding rectangle of this node.
    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    /// Computes the absolute position of an input port's connection point
    /// (one cell to the left of the node border).
    #[must_use]
    pub fn input_port_pos(&self, index: usize) -> (u16, u16) {
        #[expect(clippy::indexing_slicing, reason = "caller should use valid index")]
        let port = &self.input_ports[index];
        (self.x.saturating_sub(1), self.y + port.row_offset)
    }

    /// Computes the absolute position of an output port's connection point
    /// (one cell to the right of the node border).
    #[must_use]
    pub fn output_port_pos(&self, index: usize) -> (u16, u16) {
        #[expect(clippy::indexing_slicing, reason = "caller should use valid index")]
        let port = &self.output_ports[index];
        (self.x + self.width, self.y + port.row_offset)
    }

    /// Returns a viewport-shifted wrapper with `i32` positions for rendering.
    ///
    /// Unlike the old `shifted()` method, this uses `i32` arithmetic so nodes
    /// at `x=0` can scroll off-screen to the left/top.
    #[must_use]
    pub fn shifted_i32(&self, dx: i32, dy: i32) -> ShiftedNode<'_> {
        ShiftedNode {
            inner: self,
            x: i32::from(self.x).saturating_sub(dx),
            y: i32::from(self.y).saturating_sub(dy),
        }
    }
}

/// Sets a buffer cell at `i32` absolute coordinates, skipping negative/out-of-range.
fn set_cell_abs(buf: &mut Buffer, col: i32, row: i32, symbol: &str, style: Style) {
    if col < 0 || row < 0 {
        return;
    }
    let Ok(col) = u16::try_from(col) else {
        return;
    };
    let Ok(row) = u16::try_from(row) else {
        return;
    };
    if let Some(cell) = buf.cell_mut(Position::new(col, row)) {
        cell.set_symbol(symbol);
        cell.set_style(style);
    }
}

/// A [`VisualNode`] with viewport-shifted `i32` positions for rendering.
///
/// Produced by [`VisualNode::shifted_i32()`]. Unlike the node's native `u16`
/// positions, this can represent negative coordinates (node scrolled off-screen).
pub struct ShiftedNode<'a> {
    /// The underlying node.
    pub inner: &'a VisualNode,
    /// Shifted x position (can be negative).
    pub x: i32,
    /// Shifted y position (can be negative).
    pub y: i32,
}

impl ShiftedNode<'_> {
    /// Returns true if any part of the node is visible (not fully scrolled off-screen).
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.x.saturating_add(i32::from(self.inner.width)) > 0
            && self.y.saturating_add(i32::from(self.inner.height)) > 0
    }

    /// Renders the node into the buffer at the shifted position.
    ///
    /// If the node has `AwaitingInput` status, the border uses `awaiting_input_color`
    /// instead of the default selected/unselected color.
    pub fn render(&self, buf: &mut Buffer, selected: bool, tick: u8, awaiting_input_color: Color) {
        let border_color = if selected {
            Color::White
        } else if self.inner.status == NodeStatus::AwaitingInput {
            awaiting_input_color
        } else {
            Color::DarkGray
        };
        let border_style = Style::default().fg(border_color);

        self.render_borders(buf, border_style);
        self.render_title(buf, border_style, tick);
        self.render_port_labels(buf);
        self.render_port_indicators(buf);
        self.render_status(buf, tick, awaiting_input_color);
    }

    /// Sets a cell in the buffer at the given local coordinates.
    fn set_cell(
        &self,
        buf: &mut Buffer,
        local_col: u16,
        local_row: u16,
        symbol: &str,
        style: Style,
    ) {
        let col = self.x + i32::from(local_col);
        let row = self.y + i32::from(local_row);
        set_cell_abs(buf, col, row, symbol, style);
    }

    /// Renders the rounded border around the node.
    fn render_borders(&self, buf: &mut Buffer, style: Style) {
        let width = usize::from(self.inner.width);
        let height = usize::from(self.inner.height);

        // Top border: ╭─...─╮
        self.set_cell(buf, 0, 0, "╭", style);
        for col in 1..width.saturating_sub(1) {
            self.set_cell(buf, u16::try_from(col).unwrap_or(u16::MAX), 0, "─", style);
        }
        self.set_cell(buf, self.inner.width.saturating_sub(1), 0, "╮", style);

        // Side borders: │ ... │
        for row in 1..height.saturating_sub(1) {
            self.set_cell(buf, 0, u16::try_from(row).unwrap_or(u16::MAX), "│", style);
            self.set_cell(
                buf,
                self.inner.width.saturating_sub(1),
                u16::try_from(row).unwrap_or(u16::MAX),
                "│",
                style,
            );
        }

        // Bottom border: ╰─...─╯
        let last_row = self.inner.height.saturating_sub(1);
        self.set_cell(buf, 0, last_row, "╰", style);
        for col in 1..width.saturating_sub(1) {
            self.set_cell(
                buf,
                u16::try_from(col).unwrap_or(u16::MAX),
                last_row,
                "─",
                style,
            );
        }
        self.set_cell(
            buf,
            self.inner.width.saturating_sub(1),
            last_row,
            "╯",
            style,
        );
    }

    /// Renders the node title text on the top border.
    fn render_title(&self, buf: &mut Buffer, border_style: Style, _tick: u8) {
        let start_col = 2;
        let max_title_len = usize::from(self.inner.width).saturating_sub(4);
        let title = truncate_str(&self.inner.name, max_title_len);
        for (i, g) in title.graphemes(true).enumerate() {
            let col = start_col + u16::try_from(i).unwrap_or(u16::MAX);
            if col < self.inner.width.saturating_sub(2) {
                self.set_cell(buf, col, 0, g, border_style);
            }
        }
    }

    /// Renders the status indicator on the top border.
    fn render_status(&self, buf: &mut Buffer, tick: u8, awaiting_input_color: Color) {
        let symbol = status_symbol(self.inner.status, tick);
        let color = status_color(self.inner.status, awaiting_input_color);
        let col = self.inner.width.saturating_sub(2);
        self.set_cell(buf, col, 0, symbol, Style::default().fg(color));
    }

    /// Renders the port name labels inside the node body.
    fn render_port_labels(&self, buf: &mut Buffer) {
        let content_start = 1 + H_PAD;
        let content_width = usize::from(self.inner.width).saturating_sub(2 + 2 * H_PAD);

        for port in &self.inner.input_ports {
            let label = format!("{} {}", port_type_label(port.port_type), port.name);
            let label = truncate_str(&label, content_width);
            let style = Style::default().fg(port_type_color(port.port_type));
            self.render_text(buf, content_start, port.row_offset, &label, style);
        }

        for port in &self.inner.output_ports {
            let label = format!("{} {}", port_type_label(port.port_type), port.name);
            let label = truncate_str(&label, content_width);
            let style = Style::default().fg(port_type_color(port.port_type));
            let label_len = label.graphemes(true).count();
            let start_col = (content_start + content_width)
                .saturating_sub(label_len)
                .max(content_start);
            self.render_text(buf, start_col, port.row_offset, &label, style);
        }
    }

    /// Renders the port indicator circles on the node borders.
    fn render_port_indicators(&self, buf: &mut Buffer) {
        let style = Style::default().add_modifier(Modifier::BOLD);

        for port in &self.inner.input_ports {
            let color = port_type_color(port.port_type);
            self.set_cell(buf, 0, port.row_offset, "○", style.fg(color));
        }

        for port in &self.inner.output_ports {
            let color = port_type_color(port.port_type);
            self.set_cell(
                buf,
                self.inner.width.saturating_sub(1),
                port.row_offset,
                "○",
                style.fg(color),
            );
        }
    }

    /// Renders a text string at the given position within the node.
    fn render_text(&self, buf: &mut Buffer, start_col: usize, row: u16, text: &str, style: Style) {
        for (i, g) in text.graphemes(true).enumerate() {
            let col = u16::try_from(start_col + i).unwrap_or(u16::MAX);
            if col < self.inner.width.saturating_sub(1) {
                self.set_cell(buf, col, row, g, style);
            }
        }
    }
}

/// Truncates a string to at most `max_len` grapheme clusters.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.graphemes(true).count() <= max_len {
        s.to_owned()
    } else {
        s.graphemes(true).take(max_len).collect()
    }
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "test assertions use known-valid indices"
)]
mod tests {
    use super::*;

    fn string_port(name: &'static str) -> PortDef {
        PortDef::text(name)
    }

    fn json_port(name: &'static str) -> PortDef {
        PortDef::json(name)
    }

    #[test]
    fn node_with_two_inputs_one_output_has_correct_dimensions() {
        // Given a node with 2 inputs and 1 output.
        let node = VisualNode::compute(
            "foo".to_owned(),
            &[string_port("prompt"), json_port("config")],
            &[string_port("result")],
            NodeStatus::Pending,
        );

        // Then width fits the longest label "String prompt" = 13 + 2 pad + 2 border = 17.
        // Actually: content_width = max("foo ".len(), max("String prompt".len(), "String result".len(), "Json config".len())) = 14
        // width = 14 + 2 + 2 = 18
        assert!(
            node.width >= 14,
            "width should fit content, got {}",
            node.width
        );
        // height = 1 + 2 + 1 + 1 + 1 = 6
        assert_eq!(node.height, 6);
    }

    #[test]
    fn source_node_has_gap_row() {
        // Given a source node (0 inputs, 1 output).
        let node = VisualNode::compute(
            "source".to_owned(),
            &[],
            &[string_port("out")],
            NodeStatus::Pending,
        );

        // Then height = 1 + max(0,1) + 1 + 1 + 1 = 5.
        assert_eq!(node.height, 5);
        // The gap row exists at row 1 (no inputs, but gap still present).
        // Output port should be at row 3 (height - 2 = 5 - 2 = 3).
        assert_eq!(node.output_ports[0].row_offset, 3);
    }

    #[test]
    fn sink_node_has_gap_row() {
        // Given a sink node (1 input, 0 outputs).
        let node = VisualNode::compute(
            "sink".to_owned(),
            &[string_port("in")],
            &[],
            NodeStatus::Pending,
        );

        // Then height = 1 + 1 + 1 + max(0,1) + 1 = 5.
        assert_eq!(node.height, 5);
        // Input port at row 1.
        assert_eq!(node.input_ports[0].row_offset, 1);
        // Gap row at row 2.
    }

    #[test]
    fn input_ports_top_aligned_output_ports_bottom_aligned() {
        // Given a node with 2 inputs and 2 outputs.
        let node = VisualNode::compute(
            "node".to_owned(),
            &[string_port("a"), string_port("b")],
            &[string_port("x"), string_port("y")],
            NodeStatus::Pending,
        );

        // height = 1 + 2 + 1 + 2 + 1 = 7
        assert_eq!(node.height, 7);

        // Inputs: row 1, row 2 (top-aligned).
        assert_eq!(node.input_ports[0].row_offset, 1);
        assert_eq!(node.input_ports[1].row_offset, 2);

        // Outputs: bottom-aligned: row 5, row 4 (bottom-up).
        assert_eq!(node.output_ports[0].row_offset, 5);
        assert_eq!(node.output_ports[1].row_offset, 4);
    }

    #[test]
    fn renders_rounded_corners() {
        // Given a simple node.
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );

        // When rendering to a buffer.
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 15,
        };
        let mut buf = Buffer::empty(area);
        node.render(&mut buf, false, 0, Color::Cyan);

        // Then rounded corners are present.
        assert_eq!(buf.cell(Position::new(0, 0)).unwrap().symbol(), "╭");
        assert_eq!(
            buf.cell(Position::new(node.width.saturating_sub(1), 0))
                .unwrap()
                .symbol(),
            "╮"
        );
        assert_eq!(
            buf.cell(Position::new(0, node.height.saturating_sub(1)))
                .unwrap()
                .symbol(),
            "╰"
        );
        assert_eq!(
            buf.cell(Position::new(
                node.width.saturating_sub(1),
                node.height.saturating_sub(1),
            ))
            .unwrap()
            .symbol(),
            "╯"
        );
    }

    #[test]
    fn port_indicators_on_borders() {
        // Given a node with 1 input and 1 output.
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );

        // When rendering.
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 15,
        };
        let mut buf = Buffer::empty(area);
        node.render(&mut buf, false, 0, Color::Cyan);

        // Then port indicators appear at correct positions.
        // Input port at (0, input_ports[0].row_offset).
        let input_row = node.input_ports[0].row_offset;
        assert_eq!(buf.cell(Position::new(0, input_row)).unwrap().symbol(), "○");

        // Output port at (width-1, output_ports[0].row_offset).
        let output_row = node.output_ports[0].row_offset;
        assert_eq!(
            buf.cell(Position::new(node.width.saturating_sub(1), output_row))
                .unwrap()
                .symbol(),
            "○"
        );
    }

    #[test]
    fn status_indicator_shows_pending_circle() {
        // Given a pending node.
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );

        // When rendering.
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 15,
        };
        let mut buf = Buffer::empty(area);
        node.render(&mut buf, false, 0, Color::Cyan);

        // Then status indicator at (width-2, 0) shows ○.
        let col = node.width.saturating_sub(2);
        assert_eq!(buf.cell(Position::new(col, 0)).unwrap().symbol(), "○");
    }

    #[test]
    fn status_indicator_shows_completed_circle() {
        // Given a completed node.
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Completed,
        );

        // When rendering.
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 15,
        };
        let mut buf = Buffer::empty(area);
        node.render(&mut buf, false, 0, Color::Cyan);

        // Then status indicator shows ●.
        let col = node.width.saturating_sub(2);
        assert_eq!(buf.cell(Position::new(col, 0)).unwrap().symbol(), "●");
    }

    // Kills: truncate_str -> String::new(), truncate_str -> "xyzzy"
    #[test]
    fn truncate_str_returns_actual_content() {
        let result = truncate_str("hello world", 5);
        assert_eq!(
            result, "hello",
            "must return actual truncated content, not empty or xyzzy"
        );
    }

    #[test]
    fn truncate_str_returns_full_string_when_short_enough() {
        let result = truncate_str("hi", 10);
        assert_eq!(result, "hi", "short strings pass through unchanged");
    }

    // Kills: ShiftedNode::is_visible -> true
    #[test]
    fn shifted_node_invisible_when_scrolled_left() {
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );
        // Shift the node so it's entirely off-screen to the left.
        // Node width is ~18. With dx = 100, shifted x = 0 - 100 = -100.
        let shifted = node.shifted_i32(100, 0);
        assert!(
            !shifted.is_visible(),
            "node scrolled off-screen left must be invisible"
        );
    }

    // Kills: ShiftedNode::is_visible -> true
    #[test]
    fn shifted_node_invisible_when_scrolled_up() {
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );
        let shifted = node.shifted_i32(0, 100);
        assert!(
            !shifted.is_visible(),
            "node scrolled off-screen up must be invisible"
        );
    }

    // Kills: ShiftedNode::is_visible && -> ||
    #[test]
    fn shifted_node_visible_when_in_viewport() {
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );
        let shifted = node.shifted_i32(0, 0);
        assert!(shifted.is_visible(), "node at origin must be visible");
    }

    // Kills: VisualNode::rect() returning default
    #[test]
    fn visual_node_rect_returns_actual_dimensions() {
        let node = VisualNode::compute(
            "my_node".to_owned(),
            &[string_port("a")],
            &[string_port("b")],
            NodeStatus::Pending,
        );
        let rect = node.rect();
        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.width, node.width);
        assert_eq!(rect.height, node.height);
        assert!(rect.width > 0, "width must be non-zero");
        assert!(rect.height > 0, "height must be non-zero");
    }

    // Kills: input_port_pos uses saturating_sub correctly
    #[test]
    fn input_port_pos_is_left_of_node() {
        let mut node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );
        node.x = 5;
        node.y = 10;
        let (px, py) = node.input_port_pos(0);
        assert_eq!(px, 4, "input port x must be node.x - 1");
        assert_eq!(
            py,
            10 + node.input_ports[0].row_offset,
            "input port y must be node.y + row_offset"
        );
    }

    // Kills: output_port_pos uses + correctly (not * or -)
    #[test]
    fn output_port_pos_is_right_of_node() {
        let mut node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );
        node.x = 5;
        node.y = 10;
        let (px, py) = node.output_port_pos(0);
        assert_eq!(px, 5 + node.width, "output port x must be node.x + width");
        assert_eq!(
            py,
            10 + node.output_ports[0].row_offset,
            "output port y must be node.y + row_offset"
        );
    }

    // Kills: set_cell_abs skipping negative coords
    #[test]
    fn set_cell_abs_skips_negative_coords() {
        let area = Rect::new(0, 0, 20, 20);
        let mut buf = Buffer::empty(area);
        set_cell_abs(&mut buf, -1, 5, "X", Style::default());
        set_cell_abs(&mut buf, 5, -1, "Y", Style::default());
        // Both should be skipped - check that cell at (5,5) is still default.
        let cell = buf.cell(Position::new(5, 5)).unwrap();
        assert_eq!(cell.symbol(), " ", "negative coords must be skipped");
    }

    // Kills: set_cell_abs writes valid coords
    #[test]
    fn set_cell_abs_writes_to_valid_position() {
        let area = Rect::new(0, 0, 20, 20);
        let mut buf = Buffer::empty(area);
        set_cell_abs(&mut buf, 5, 5, "X", Style::default().fg(Color::Red));
        let cell = buf.cell(Position::new(5, 5)).unwrap();
        assert_eq!(cell.symbol(), "X");
        assert_eq!(cell.fg, Color::Red);
    }

    // Kills: node.render with selected=true uses white border
    #[test]
    fn render_selected_node_has_white_border() {
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );
        let area = Rect::new(0, 0, 30, 15);
        let mut buf = Buffer::empty(area);
        node.render(&mut buf, true, 0, Color::Cyan);
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(cell.fg, Color::White, "selected node border must be white");
    }

    // Kills: node.render with selected=false and non-awaiting uses DarkGray border
    #[test]
    fn render_unselected_node_has_dark_gray_border() {
        let node = VisualNode::compute(
            "test".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );
        let area = Rect::new(0, 0, 30, 15);
        let mut buf = Buffer::empty(area);
        node.render(&mut buf, false, 0, Color::Cyan);
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(
            cell.fg,
            Color::DarkGray,
            "unselected node border must be dark gray"
        );
    }

    // Kills: render_title writes actual title content
    #[test]
    fn render_title_writes_name_to_buffer() {
        let node = VisualNode::compute(
            "my_node".to_owned(),
            &[string_port("in")],
            &[string_port("out")],
            NodeStatus::Pending,
        );
        let area = Rect::new(0, 0, 30, 15);
        let mut buf = Buffer::empty(area);
        node.render(&mut buf, false, 0, Color::Cyan);
        // Title starts at col 2.
        let cell = buf.cell(Position::new(2, 0)).unwrap();
        assert_eq!(cell.symbol(), "m", "title must render actual name text");
    }
}
