//! The workflow visualization widget for ratatui.
//!
//! [`WorkflowWidget`] implements [`ratatui::widgets::Widget`] and renders an entire
//! workflow graph — nodes, connections, status indicators — into a ratatui buffer.

use std::collections::HashMap;

use jinn_workflow::execution::ExecutionSnapshot;
use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

use crate::connection::{
    CellInfo, ConnectionRouter, SimpleRouter, insert_path_into_grid, render_merged_grid,
};
use crate::layout::{self, GraphLayout};
use crate::node::VisualNode;
use crate::viewport::ViewportState;

/// The workflow visualization widget.
///
/// Constructed fresh each frame (standard ratatui pattern). Renders the entire
/// graph — nodes with status indicators, typed ports, and L-shaped connections.
pub struct WorkflowWidget<'a> {
    /// Workflow execution snapshot to render.
    snapshot: &'a ExecutionSnapshot,
    /// Viewport state for pan/zoom and selection.
    viewport: &'a ViewportState,
    /// Animation tick counter for spinner frames.
    tick: u8,
    /// Color to use for AwaitingInput node borders and indicators.
    awaiting_input_color: Color,
}

impl<'a> WorkflowWidget<'a> {
    /// Creates a new workflow widget.
    #[must_use]
    pub fn new(
        snapshot: &'a ExecutionSnapshot,
        viewport: &'a ViewportState,
        tick: u8,
        awaiting_input_color: Color,
    ) -> Self {
        Self {
            snapshot,
            viewport,
            tick,
            awaiting_input_color,
        }
    }

    /// Builds a node name → VisualNode lookup from the layout.
    fn build_node_map(layout: &GraphLayout) -> HashMap<&str, &VisualNode> {
        layout.nodes.iter().map(|n| (n.name.as_str(), n)).collect()
    }

    /// Renders all connections in the graph.
    fn render_connections(
        &self,
        buf: &mut Buffer,
        layout: &GraphLayout,
        node_map: &HashMap<&str, &VisualNode>,
        area: Rect,
    ) {
        let node_rects: Vec<Rect> = layout
            .nodes
            .iter()
            .map(super::node::VisualNode::rect)
            .collect();
        let mut grid: HashMap<(i32, i32), CellInfo> = HashMap::new();

        // Area origin offset for connection paths (same as node rendering).
        let ox = i32::from(area.x);
        let oy = i32::from(area.y);

        for edge in self.snapshot.structure().edges() {
            // Find the output port position on the source node.
            let Some(src_node) = node_map.get(edge.source_node.as_str()) else {
                continue;
            };
            let src_port_idx = src_node
                .output_ports
                .iter()
                .position(|p| p.name == edge.source_port);
            let Some(src_idx) = src_port_idx else {
                continue;
            };
            let (sx, sy) = src_node.output_port_pos(src_idx);

            // Find the input port position on the target node.
            let Some(tgt_node) = node_map.get(edge.target_node.as_str()) else {
                continue;
            };
            let tgt_port_idx = tgt_node
                .input_ports
                .iter()
                .position(|p| p.name == edge.target_port);
            let Some(tgt_idx) = tgt_port_idx else {
                continue;
            };
            let (tx, ty) = tgt_node.input_port_pos(tgt_idx);

            // Apply viewport offset and area origin using i32 arithmetic.
            let sx = i32::from(sx)
                .saturating_sub(self.viewport.offset_x)
                .saturating_add(ox);
            let sy = i32::from(sy)
                .saturating_sub(self.viewport.offset_y)
                .saturating_add(oy);
            let tx = i32::from(tx)
                .saturating_sub(self.viewport.offset_x)
                .saturating_add(ox);
            let ty = i32::from(ty)
                .saturating_sub(self.viewport.offset_y)
                .saturating_add(oy);

            let path = SimpleRouter::route((sx, sy), (tx, ty), &node_rects);
            insert_path_into_grid(&mut grid, &path, edge.port_type);
        }

        render_merged_grid(buf, &grid, area);
    }
}

impl Widget for WorkflowWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Compute layout.
        let layout = layout::compute(self.snapshot);

        if layout.nodes.is_empty() {
            return;
        }

        let node_map = Self::build_node_map(&layout);

        // Render connections first (so nodes draw on top).
        self.render_connections(buf, &layout, &node_map, area);

        // Area origin offset — layout positions are relative to the content area,
        // but the buffer has absolute coordinates. Add area.x/area.y to translate.
        let ox = i32::from(area.x);
        let oy = i32::from(area.y);

        // Render each node.
        for node in &layout.nodes {
            let selected = self.viewport.is_selected(&node.name);
            let mut shifted = node.shifted_i32(self.viewport.offset_x, self.viewport.offset_y);
            shifted.x = shifted.x.saturating_add(ox);
            shifted.y = shifted.y.saturating_add(oy);
            if !shifted.is_visible() {
                continue;
            }
            shifted.render(buf, selected, self.tick, self.awaiting_input_color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jinn_workflow::execution::WorkflowExecution;
    use jinn_workflow::graph::WorkflowGraph;
    use jinn_workflow::graph::WorkflowGraphBuilder;
    use jinn_workflow::node::{NodeContext, NodeError, WorkflowNode};
    use jinn_workflow::port::{PortDef, PortValues};

    struct TestNode {
        name: &'static str,
        inputs: Vec<PortDef>,
        outputs: Vec<PortDef>,
    }

    impl TestNode {
        fn source(name: &'static str) -> Self {
            Self {
                name,
                inputs: vec![],
                outputs: vec![PortDef::text("out")],
            }
        }
        fn sink(name: &'static str) -> Self {
            Self {
                name,
                inputs: vec![PortDef::text("in")],
                outputs: vec![],
            }
        }
        fn passthrough(name: &'static str) -> Self {
            Self {
                name,
                inputs: vec![PortDef::text("in")],
                outputs: vec![PortDef::text("out")],
            }
        }
        fn merge_sink(name: &'static str) -> Self {
            Self {
                name,
                inputs: vec![PortDef::text("in_1"), PortDef::text("in_2")],
                outputs: vec![],
            }
        }
    }

    #[expect(dead_code, reason = "test helper implementing trait")]
    struct TestContext;
    impl NodeContext for TestContext {}

    #[async_trait::async_trait]
    impl WorkflowNode for TestNode {
        fn name(&self) -> &str {
            self.name
        }
        fn input_ports(&self) -> Vec<PortDef> {
            self.inputs.clone()
        }
        fn output_ports(&self) -> Vec<PortDef> {
            self.outputs.clone()
        }
        async fn execute(
            &self,
            _inputs: PortValues,
            _ctx: &dyn NodeContext,
        ) -> Result<PortValues, error_stack::Report<NodeError>> {
            Ok(PortValues::new())
        }
        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(Self {
                name: self.name,
                inputs: self.inputs.clone(),
                outputs: self.outputs.clone(),
            })
        }
    }

    fn build_two_node_graph() -> WorkflowGraph {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node("src".to_owned(), Box::new(TestNode::source("src")));
        b.add_node("snk".to_owned(), Box::new(TestNode::sink("snk")));
        b.connect("src", "out", "snk", "in").unwrap();
        b.build().unwrap()
    }

    #[test]
    fn widget_renders_graph_without_panic() {
        let graph = build_two_node_graph();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let viewport = ViewportState::new();
        let widget = WorkflowWidget::new(&snapshot, &viewport, 0, Color::Cyan);

        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Just verify no panic occurred — the real test is that render completed.
    }

    #[test]
    fn widget_renders_empty_graph_without_panic() {
        let graph = WorkflowGraphBuilder::new().build().unwrap_or_else(|_| {
            // An empty graph builder may fail. Use a single-node graph instead.
            let mut b = WorkflowGraphBuilder::new();
            b.add_node("empty".to_owned(), Box::new(TestNode::source("empty")));
            b.build().unwrap()
        });
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let viewport = ViewportState::new();
        let widget = WorkflowWidget::new(&snapshot, &viewport, 0, Color::Cyan);

        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
    }

    #[test]
    fn widget_with_selected_node_renders() {
        let graph = build_two_node_graph();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let viewport = ViewportState::with_selected("src".to_owned());
        let widget = WorkflowWidget::new(&snapshot, &viewport, 0, Color::Cyan);

        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Just verify no panic occurred and render completed.
        let _ = buf.area;
    }

    #[test]
    fn widget_renders_running_spinner() {
        let graph = build_two_node_graph();
        let execution = WorkflowExecution::new(graph);
        execution.set_status("src", jinn_workflow::engine::NodeStatus::Running);
        let snapshot = execution.snapshot();
        let viewport = ViewportState::new();

        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let mut buf = Buffer::empty(area);

        // Render with tick=0.
        let w = WorkflowWidget::new(&snapshot, &viewport, 0, Color::Cyan);
        w.render(area, &mut buf);

        // The spinner frame for tick=0 is ⠋. Check it appears somewhere.
        let mut has_spinner = false;
        for row in 0..area.height {
            for col in 0..area.width {
                if let Some(cell) = buf.cell(ratatui::layout::Position::new(col, row))
                    && cell.symbol() == "\u{280b}"
                {
                    has_spinner = true;
                }
            }
        }
        assert!(has_spinner, "should find spinner frame ⠋ for Running node");
    }

    #[test]
    fn widget_diamond_graph_has_tee_junctions() {
        // Given a diamond graph: source → b, c → merge_sink.
        let mut b = WorkflowGraphBuilder::new();
        b.add_node("a".to_owned(), Box::new(TestNode::source("a")));
        b.add_node("b".to_owned(), Box::new(TestNode::passthrough("b")));
        b.add_node("c".to_owned(), Box::new(TestNode::passthrough("c")));
        b.add_node("d".to_owned(), Box::new(TestNode::merge_sink("d")));
        b.connect("a", "out", "b", "in").unwrap();
        b.connect("a", "out", "c", "in").unwrap();
        b.connect("b", "out", "d", "in_1").unwrap();
        b.connect("c", "out", "d", "in_2").unwrap();
        let graph = b.build().unwrap();

        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let viewport = ViewportState::new();
        let widget = WorkflowWidget::new(&snapshot, &viewport, 0, Color::Cyan);

        let area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 30,
        };
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Then at least one tee junction character appears in the buffer.
        let has_tee = (0..area.height).any(|row| {
            (0..area.width).any(|col| {
                let cell = buf.cell(ratatui::layout::Position::new(col, row)).unwrap();
                matches!(cell.symbol(), "┬" | "┴" | "├" | "┤" | "┼")
            })
        });
        assert!(
            has_tee,
            "diamond graph should have at least one tee junction"
        );
    }

    // --- Mutant-killing tests for widget.rs ---

    // Kills: render_connections == -> != for port matching
    #[test]
    fn widget_renders_connections_for_correct_ports() {
        // Build a graph with specific port names so we can verify the right ports are connected.
        let graph = build_two_node_graph();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let viewport = ViewportState::new();
        let widget = WorkflowWidget::new(&snapshot, &viewport, 0, Color::Cyan);

        let area = Rect { x: 0, y: 0, width: 120, height: 30 };
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Verify at least one connection character appears (─ or │ or corner).
        let has_connection = (0..area.height).any(|row| {
            (0..area.width).any(|col| {
                let sym = buf.cell(ratatui::layout::Position::new(col, row)).unwrap().symbol();
                matches!(sym, "─" | "│" | "╭" | "╮" | "╰" | "╯")
            })
        });
        assert!(has_connection, "two-node graph must render connection lines");
    }
}
