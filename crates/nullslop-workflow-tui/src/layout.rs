//! Graph layout — computes 2D positions for all nodes in a workflow graph.
//!
//! Uses topological column assignment: source nodes in column 0, each downstream
//! node in `max(parent_columns) + 1`. Nodes within a column are stacked vertically.

use std::collections::HashMap;

use nullslop_workflow::engine::NodeStatus;
use nullslop_workflow::execution::{ExecutionSnapshot, WorkflowStructure};

use crate::node::VisualNode;

/// Horizontal spacing between columns (cells).
const H_SPACING: u16 = 5;
/// Vertical spacing between nodes in the same column (cells).
const V_SPACING: u16 = 1;

/// The computed layout for an entire workflow graph.
///
/// Produced by [`compute`]. Contains positioned [`VisualNode`]s ready for rendering.
pub struct GraphLayout {
    /// All nodes with their computed positions.
    pub nodes: Vec<VisualNode>,
}

impl GraphLayout {
    /// Returns the bounding box of all nodes as `(width, height)`.
    ///
    /// Width = `max(node.x + node.width)`, height = `max(node.y + node.height)`.
    /// Returns `(0, 0)` for empty layouts.
    #[must_use]
    pub fn content_size(&self) -> (u16, u16) {
        if self.nodes.is_empty() {
            return (0, 0);
        }
        let max_x = self
            .nodes
            .iter()
            .map(|n| n.x.saturating_add(n.width))
            .max()
            .unwrap_or(0);
        let max_y = self
            .nodes
            .iter()
            .map(|n| n.y.saturating_add(n.height))
            .max()
            .unwrap_or(0);
        (max_x, max_y)
    }
}

/// Computes the layout for a workflow graph.
///
/// Assigns each node a column based on topological depth, then stacks
/// nodes within each column vertically with spacing.
///
/// # Panics
///
/// Does not panic; returns an empty layout for empty graphs.
#[must_use]
pub fn compute(snapshot: &ExecutionSnapshot) -> GraphLayout {
    let structure = snapshot.structure();
    let mut nodes = Vec::new();

    let all_names: Vec<&str> = structure.node_names().collect();
    if all_names.is_empty() {
        return GraphLayout { nodes };
    }

    let columns = compute_columns(structure);

    let mut column_nodes: HashMap<usize, Vec<&str>> = HashMap::new();
    for name in &all_names {
        let col = columns.get(*name).copied().unwrap_or(0);
        column_nodes.entry(col).or_default().push(name);
    }

    let max_col = columns.values().copied().max().unwrap_or(0);

    // First pass: compute all VisualNodes to get widths/heights.
    let mut visual_nodes: HashMap<&str, VisualNode> = HashMap::new();
    for name in &all_names {
        let input_defs = structure.node_input_ports(name).unwrap_or_default();
        let output_defs = structure.node_output_ports(name).unwrap_or_default();
        let status = snapshot.status_of(name).unwrap_or(NodeStatus::Pending);
        let node = VisualNode::compute(name.to_string(), input_defs, output_defs, status);
        visual_nodes.insert(name, node);
    }

    // Second pass: assign positions by column.
    for col in 0..=max_col {
        let Some(col_names) = column_nodes.get(&col) else {
            continue;
        };

        let x_offset = compute_x_offset(&visual_nodes, &column_nodes, col);

        let mut y_cursor: u16 = 0;
        for name in col_names {
            #[expect(clippy::expect_used, reason = "node was created in first pass")]
            let node = visual_nodes
                .get_mut(name)
                .expect("node was created in first pass");
            node.x = x_offset;
            node.y = y_cursor;
            y_cursor = y_cursor + node.height + V_SPACING;
        }
    }

    // Collect into Vec preserving insertion order (all_names).
    for name in all_names {
        if let Some(node) = visual_nodes.remove(name) {
            nodes.push(node);
        }
    }

    GraphLayout { nodes }
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

    // Also handle nodes with no incoming edges that aren't in sources()
    // (e.g., nodes with no input ports).
    for name in structure.node_names() {
        let has_inputs = structure
            .node_input_ports(name)
            .is_some_and(|ports| !ports.is_empty());
        if !has_inputs {
            columns.insert(name, 0);
        }
    }

    // Propagate: for each edge, target column = max(target column, source column + 1).
    // Iterate until stable.
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
    visual_nodes: &HashMap<&str, VisualNode>,
    column_nodes: &HashMap<usize, Vec<&str>>,
    target_col: usize,
) -> u16 {
    let mut x: u16 = 0;
    for col in 0..target_col {
        let max_width = column_nodes.get(&col).map_or(0, |names| {
            names
                .iter()
                .filter_map(|n| visual_nodes.get(n).map(|node| node.width))
                .max()
                .unwrap_or(0)
        });
        x = x + max_width + H_SPACING;
    }
    x
}

#[cfg(test)]
#[expect(clippy::indexing_slicing, reason = "test indices are known-valid")]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use nullslop_workflow::execution::WorkflowExecution;
    use nullslop_workflow::graph::WorkflowGraph;
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

    fn build_linear() -> WorkflowGraph {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node("a".to_owned(), Box::new(TestNode::source("a")));
        b.add_node("b".to_owned(), Box::new(TestNode::passthrough("b")));
        b.add_node("c".to_owned(), Box::new(TestNode::sink("c")));
        b.connect("a", "out", "b", "in").unwrap();
        b.connect("b", "out", "c", "in").unwrap();
        b.build().unwrap()
    }

    fn build_fan_out() -> WorkflowGraph {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node("a".to_owned(), Box::new(TestNode::source("a")));
        b.add_node("b".to_owned(), Box::new(TestNode::sink("b")));
        b.add_node("c".to_owned(), Box::new(TestNode::sink("c")));
        b.connect("a", "out", "b", "in").unwrap();
        b.connect("a", "out", "c", "in").unwrap();
        b.build().unwrap()
    }

    fn build_diamond() -> WorkflowGraph {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node("a".to_owned(), Box::new(TestNode::source("a")));
        b.add_node("b".to_owned(), Box::new(TestNode::passthrough("b")));
        b.add_node("c".to_owned(), Box::new(TestNode::passthrough("c")));
        b.add_node("d".to_owned(), Box::new(TestNode::merge_sink("d")));
        b.connect("a", "out", "b", "in").unwrap();
        b.connect("a", "out", "c", "in").unwrap();
        b.connect("b", "out", "d", "in_1").unwrap();
        b.connect("c", "out", "d", "in_2").unwrap();
        b.build().unwrap()
    }

    #[test]
    fn linear_graph_assigns_correct_columns() {
        let graph = build_linear();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let columns = compute_columns(snapshot.structure());
        assert_eq!(columns["a"], 0);
        assert_eq!(columns["b"], 1);
        assert_eq!(columns["c"], 2);
    }

    #[test]
    fn fan_out_nodes_share_column() {
        let graph = build_fan_out();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let columns = compute_columns(snapshot.structure());
        assert_eq!(columns["a"], 0);
        assert_eq!(columns["b"], 1);
        assert_eq!(columns["c"], 1);
    }

    #[test]
    fn diamond_assigns_correct_columns() {
        let graph = build_diamond();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let columns = compute_columns(snapshot.structure());
        assert_eq!(columns["a"], 0);
        assert_eq!(columns["b"], 1);
        assert_eq!(columns["c"], 1);
        assert_eq!(columns["d"], 2);
    }

    #[test]
    fn layout_produces_non_overlapping_positions() {
        let graph = build_diamond();
        let execution = WorkflowExecution::new(graph);
        let layout = compute(&execution.snapshot());

        for i in 0..layout.nodes.len() {
            for j in (i + 1)..layout.nodes.len() {
                let a = layout.nodes[i].rect();
                let b = layout.nodes[j].rect();
                assert!(
                    !a.intersects(b),
                    "nodes {:?} and {:?} overlap at {:?} and {:?}",
                    layout.nodes[i].name,
                    layout.nodes[j].name,
                    a,
                    b,
                );
            }
        }
    }

    #[test]
    fn layout_port_positions_are_outside_borders() {
        let graph = build_linear();
        let execution = WorkflowExecution::new(graph);
        let layout = compute(&execution.snapshot());

        for node in &layout.nodes {
            for (i, port) in node.input_ports.iter().enumerate() {
                let (px, _py) = node.input_port_pos(i);
                assert!(
                    px < node.x,
                    "input port '{}' x={px} should be < node.x={}",
                    port.name,
                    node.x,
                );
            }

            for (i, port) in node.output_ports.iter().enumerate() {
                let (px, _py) = node.output_port_pos(i);
                assert!(
                    px >= node.x + node.width,
                    "output port '{}' x={px} should be >= node.x + width = {}",
                    port.name,
                    node.x + node.width,
                );
            }
        }
    }

    #[test]
    fn layout_content_size_empty_graph() {
        // Given an empty layout.
        let layout = GraphLayout { nodes: vec![] };

        // Then content_size returns (0, 0).
        assert_eq!(layout.content_size(), (0, 0));
    }

    #[test]
    fn layout_content_size_single_node() {
        // Given a single-node layout at position (5, 10).
        let mut node = VisualNode::compute(
            "a".to_owned(),
            &[],
            &[PortDef::text("out")],
            NodeStatus::Pending,
        );
        node.x = 5;
        node.y = 10;
        let layout = GraphLayout { nodes: vec![node] };

        // Then content_size includes position + dimensions.
        let (w, h) = layout.content_size();
        assert_eq!(w, 5 + layout.nodes[0].width);
        assert_eq!(h, 10 + layout.nodes[0].height);
    }

    #[test]
    fn layout_content_size_linear_graph() {
        // Given a 3-node linear graph.
        let graph = build_linear();
        let execution = WorkflowExecution::new(graph);
        let layout = compute(&execution.snapshot());

        // Then content_size returns non-zero bounds.
        let (w, h) = layout.content_size();
        assert!(w > 0, "linear graph should have non-zero width");
        assert!(h > 0, "linear graph should have non-zero height");
    }

    #[test]
    fn layout_content_size_diamond_graph() {
        // Given a diamond graph (fan-out + fan-in).
        let graph = build_diamond();
        let execution = WorkflowExecution::new(graph);
        let layout = compute(&execution.snapshot());

        // Then content_size returns bounds larger than a single node.
        let (w, h) = layout.content_size();
        assert!(w > 0, "diamond graph should have non-zero width");
        assert!(h > 0, "diamond graph should have non-zero height");
        assert!(h > 5, "diamond graph should be taller than a single node");
    }

    // --- Mutant-killing tests for layout.rs ---

    // Kills: content_size + -> *, + -> -
    #[test]
    fn content_size_uses_addition() {
        let mut node = VisualNode::compute(
            "a".to_owned(),
            &[],
            &[PortDef::text("out")],
            NodeStatus::Pending,
        );
        node.x = 10;
        node.y = 20;
        let layout = GraphLayout { nodes: vec![node] };
        let (w, h) = layout.content_size();
        assert_eq!(
            w,
            10 + layout.nodes[0].width,
            "width = x + node_width, not x*node_width"
        );
        assert_eq!(
            h,
            20 + layout.nodes[0].height,
            "height = y + node_height, not y*node_height"
        );
    }

    // Kills: compute_y_offset + -> *, + -> -
    #[test]
    fn layout_nodes_in_same_column_are_stacked_vertically() {
        let graph = build_fan_out();
        let execution = WorkflowExecution::new(graph);
        let layout = compute(&execution.snapshot());

        // b and c are in the same column (1) and must not overlap.
        let b = layout.nodes.iter().find(|n| n.name == "b").expect("b");
        let c = layout.nodes.iter().find(|n| n.name == "c").expect("c");

        // They must have different y positions.
        assert_ne!(
            b.y, c.y,
            "nodes in same column must be at different y positions"
        );

        // The lower node must start after the upper node ends (with spacing).
        let (upper, lower) = if b.y < c.y { (b, c) } else { (c, b) };
        assert!(
            lower.y >= upper.y + upper.height + V_SPACING,
            "lower node must start after upper node + spacing: lower.y={}, upper.y+height+spacing={}",
            lower.y,
            upper.y + upper.height + V_SPACING,
        );
    }

    // Kills: compute_columns > -> ==, > -> <, > -> >=
    #[test]
    fn linear_graph_columns_are_strictly_increasing() {
        let graph = build_linear();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let columns = compute_columns(snapshot.structure());
        assert!(columns["b"] > columns["a"], "b must be strictly after a");
        assert!(columns["c"] > columns["b"], "c must be strictly after b");
    }

    // Kills: compute_x_offset + -> *, + -> -
    #[test]
    fn compute_x_offset_uses_addition_not_multiplication() {
        let graph = build_linear();
        let execution = WorkflowExecution::new(graph);
        let layout = compute(&execution.snapshot());

        let a = layout.nodes.iter().find(|n| n.name == "a").expect("a");
        let b = layout.nodes.iter().find(|n| n.name == "b").expect("b");
        let c = layout.nodes.iter().find(|n| n.name == "c").expect("c");

        // b.x must be >= a.x + a.width + H_SPACING (additive, not multiplicative).
        assert!(
            b.x >= a.x + a.width + H_SPACING,
            "b.x ({}) must be >= a.x ({}) + a.width ({}) + H_SPACING ({})",
            b.x,
            a.x,
            a.width,
            H_SPACING,
        );
        assert!(
            c.x >= b.x + b.width + H_SPACING,
            "c.x ({}) must be >= b.x ({}) + b.width ({}) + H_SPACING ({})",
            c.x,
            b.x,
            b.width,
            H_SPACING,
        );
    }
}
