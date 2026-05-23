//! The workflow visualization widget for ratatui.
//!
//! [`WorkflowWidget`] implements [`ratatui::widgets::Widget`] and renders an entire
//! workflow graph — nodes, connections, status indicators — into a ratatui buffer.

use std::collections::HashMap;

use nullslop_workflow::engine::NodeStatus;
use nullslop_workflow::graph::WorkflowGraph;
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

use crate::connection::{ConnectionRouter, SimpleRouter};
use crate::layout::{self, GraphLayout};
use crate::node::VisualNode;
use crate::viewport::ViewportState;

/// The workflow visualization widget.
///
/// Constructed fresh each frame (standard ratatui pattern). Renders the entire
/// graph — nodes with status indicators, typed ports, and L-shaped connections.
pub struct WorkflowWidget<'a> {
    graph: &'a WorkflowGraph,
    statuses: &'a HashMap<String, NodeStatus>,
    viewport: &'a ViewportState,
    tick: u8,
}

impl<'a> WorkflowWidget<'a> {
    /// Creates a new workflow widget.
    #[must_use]
    pub fn new(
        graph: &'a WorkflowGraph,
        statuses: &'a HashMap<String, NodeStatus>,
        viewport: &'a ViewportState,
        tick: u8,
    ) -> Self {
        Self {
            graph,
            statuses,
            viewport,
            tick,
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
        let node_rects: Vec<Rect> = layout.nodes.iter().map(|n| n.rect()).collect();

        for edge in self.graph.edges() {
            // Find the output port position on the source node.
            let Some(src_node) = node_map.get(edge.source_node) else {
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
            let Some(tgt_node) = node_map.get(edge.target_node) else {
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

            // Apply viewport offset using i32 arithmetic.
            let sx = i32::from(sx) - i32::from(self.viewport.offset_x);
            let sy = i32::from(sy) - i32::from(self.viewport.offset_y);
            let tx = i32::from(tx) - i32::from(self.viewport.offset_x);
            let ty = i32::from(ty) - i32::from(self.viewport.offset_y);

            let path = SimpleRouter::route((sx, sy), (tx, ty), &node_rects);
            crate::connection::render_path(buf, &path, edge.port_type, area);
        }
    }
}

impl Widget for WorkflowWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Compute layout.
        let layout = layout::compute(self.graph, self.statuses);

        if layout.nodes.is_empty() {
            return;
        }

        let node_map = Self::build_node_map(&layout);

        // Render connections first (so nodes draw on top).
        self.render_connections(buf, &layout, &node_map, area);

        // Render each node.
        for node in &layout.nodes {
            let selected = self.viewport.is_selected(&node.name);
            let shifted = node.shifted_i32(self.viewport.offset_x, self.viewport.offset_y);
            if !shifted.is_visible() {
                continue;
            }
            shifted.render(buf, selected, self.tick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nullslop_workflow::graph::WorkflowGraphBuilder;
    use nullslop_workflow::node::{NodeContext, NodeError, WorkflowNode};
    use nullslop_workflow::port::{PortDef, PortValues};

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
                outputs: vec![PortDef::string("out")],
            }
        }
        fn sink(name: &'static str) -> Self {
            Self {
                name,
                inputs: vec![PortDef::string("in")],
                outputs: vec![],
            }
        }
    }

    struct TestContext;
    impl NodeContext for TestContext {}

    #[async_trait::async_trait]
    impl WorkflowNode for TestNode {
        fn name(&self) -> &'static str {
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

    fn all_pending(graph: &WorkflowGraph) -> HashMap<String, NodeStatus> {
        graph
            .node_names()
            .map(|n| (n.to_owned(), NodeStatus::Pending))
            .collect()
    }

    #[test]
    fn widget_renders_graph_without_panic() {
        let graph = build_two_node_graph();
        let statuses = all_pending(&graph);
        let viewport = ViewportState::new();
        let widget = WorkflowWidget::new(&graph, &statuses, &viewport, 0);

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
        let statuses = HashMap::new();
        let viewport = ViewportState::new();
        let widget = WorkflowWidget::new(&graph, &statuses, &viewport, 0);

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
        let statuses = all_pending(&graph);
        let viewport = ViewportState::with_selected("src".to_owned());
        let widget = WorkflowWidget::new(&graph, &statuses, &viewport, 0);

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
        let mut statuses = all_pending(&graph);
        statuses.insert("src".to_owned(), NodeStatus::Running);
        let viewport = ViewportState::new();

        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let mut buf = Buffer::empty(area);

        // Render with tick=0.
        let w = WorkflowWidget::new(&graph, &statuses, &viewport, 0);
        w.render(area, &mut buf);

        // The spinner frame for tick=0 is ⠋. Check it appears somewhere.
        let mut has_spinner = false;
        for row in 0..area.height {
            for col in 0..area.width {
                if let Some(cell) = buf.cell(ratatui::layout::Position::new(col, row)) {
                    if cell.symbol() == "\u{280b}" {
                        has_spinner = true;
                    }
                }
            }
        }
        assert!(has_spinner, "should find spinner frame ⠋ for Running node");
    }
}
