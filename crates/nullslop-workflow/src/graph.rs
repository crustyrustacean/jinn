//! Graph construction and validation.
//!
//! A [`WorkflowGraph`] is a validated DAG of [`WorkflowNode`](crate::node::WorkflowNode)s
//! connected by typed port edges. Constructed via [`WorkflowGraphBuilder`], which validates
//! node names, port names, port type compatibility, cycle-free topology, and full connectivity.

use std::collections::HashMap;

use derive_more::Display;
use error_stack::Report;
use petgraph::algo::is_cyclic_directed;
use petgraph::graph::DiGraph;
use petgraph::visit::EdgeRef;
use wherror::Error;

use crate::node::WorkflowNode;
use crate::port::{PortDef, PortType};

/// Internal storage for a node in the graph.
pub(crate) struct NodeData {
    /// Node name.
    #[expect(dead_code, reason = "used for graph introspection")]
    name: String,
    /// The workflow node implementation.
    pub(crate) node: Box<dyn WorkflowNode>,
}

/// Internal storage for an edge in the graph.
#[derive(Debug)]
pub(crate) struct EdgeData {
    /// The output port name on the source node.
    pub(crate) source_port: String,
    /// The input port name on the target node.
    pub(crate) target_port: String,
    /// The validated type flowing through this edge.
    pub(crate) port_type: PortType,
}

/// A validated workflow DAG.
///
/// Constructed via [`WorkflowGraphBuilder`]. Once built, the graph is immutable
/// and ready for execution by the engine.
pub struct WorkflowGraph {
    /// The underlying petgraph DiGraph.
    inner: DiGraph<NodeData, EdgeData>,
    /// Map from node name to petgraph node index.
    name_to_index: HashMap<String, petgraph::graph::NodeIndex>,
    /// Reverse map from petgraph node index to node name.
    index_to_name: HashMap<petgraph::graph::NodeIndex, String>,
    /// Names of nodes that are graph entry points (no incoming edges).
    sources: Vec<String>,
    /// Names of nodes that are graph exit points (no outgoing edges).
    sinks: Vec<String>,
}

impl WorkflowGraph {
    /// Returns the names of source nodes (entry points with no incoming edges).
    #[must_use]
    pub fn sources(&self) -> &[String] {
        &self.sources
    }

    /// Returns the names of sink nodes (exit points with no outgoing edges).
    #[must_use]
    pub fn sinks(&self) -> &[String] {
        &self.sinks
    }

    /// Returns the number of nodes in the graph.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Returns the number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }

    /// Returns a reference to the inner petgraph.
    ///
    /// Used by the engine to traverse the graph during execution.
    pub(crate) fn inner(&self) -> &DiGraph<NodeData, EdgeData> {
        &self.inner
    }

    /// Returns a reference to the name-to-index map.
    pub(crate) fn name_to_index(&self) -> &HashMap<String, petgraph::graph::NodeIndex> {
        &self.name_to_index
    }

    /// Returns an iterator over all node names in the graph.
    ///
    /// Names are returned in the order they were added to the builder.
    pub fn node_names(&self) -> impl Iterator<Item = &str> {
        self.name_to_index.keys().map(String::as_str)
    }

    /// Returns the input port definitions for a named node.
    ///
    /// Returns `None` if no node with the given name exists.
    pub fn node_input_ports(&self, name: &str) -> Option<Vec<PortDef>> {
        let idx = self.name_to_index.get(name)?;
        Some(self.inner[*idx].node.input_ports())
    }

    /// Returns the output port definitions for a named node.
    ///
    /// Returns `None` if no node with the given name exists.
    pub fn node_output_ports(&self, name: &str) -> Option<Vec<PortDef>> {
        let idx = self.name_to_index.get(name)?;
        Some(self.inner[*idx].node.output_ports())
    }

    /// Returns an iterator over all edges in the graph.
    ///
    /// Each edge is described by its source node/port, target node/port,
    /// and the [`PortType`] flowing through it.
    pub fn edges(&self) -> impl Iterator<Item = EdgeInfo<'_>> {
        self.inner.edge_references().map(|edge| {
            let src_idx = edge.source();
            let tgt_idx = edge.target();
            #[expect(
                clippy::indexing_slicing,
                reason = "index is from petgraph edge, always valid"
            )]
            let src_name = &self.index_to_name[&src_idx];
            #[expect(
                clippy::indexing_slicing,
                reason = "index is from petgraph edge, always valid"
            )]
            let tgt_name = &self.index_to_name[&tgt_idx];
            EdgeInfo {
                source_node: src_name.as_str(),
                source_port: edge.weight().source_port.as_str(),
                target_node: tgt_name.as_str(),
                target_port: edge.weight().target_port.as_str(),
                port_type: edge.weight().port_type,
            }
        })
    }
}

/// Public information about an edge in the graph.
///
/// Returned by [`WorkflowGraph::edges()`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeInfo<'a> {
    /// Source node name.
    pub source_node: &'a str,
    /// Source port name.
    pub source_port: &'a str,
    /// Target node name.
    pub target_node: &'a str,
    /// Target port name.
    pub target_port: &'a str,
    /// The port type flowing through this edge.
    pub port_type: PortType,
}

/// Errors arising during graph construction and validation.
#[derive(Debug, Error, Display)]
pub enum GraphError {
    /// A node with the given name was not found.
    #[display("node '{name}' not found")]
    NodeNotFound {
        /// The node name that was requested.
        name: String,
    },
    /// A port with the given name was not found on a node.
    #[display("port '{port}' not found on node '{node}'")]
    PortNotFound {
        /// The node name.
        node: String,
        /// The port name.
        port: String,
    },
    /// Port types don't match on a connection.
    #[display(
        "type mismatch: node '{source_node}' port '{source_port}' ({source_type:?}) \
         cannot connect to node '{target_node}' port '{target_port}' ({target_type:?})"
    )]
    TypeMismatch {
        /// Source node name.
        source_node: String,
        /// Source port name.
        source_port: String,
        /// Source port type.
        source_type: PortType,
        /// Target node name.
        target_node: String,
        /// Target port name.
        target_port: String,
        /// Target port type.
        target_type: PortType,
    },
    /// The graph contains a cycle.
    #[display("graph contains a cycle")]
    CycleDetected,
    /// An input port has no incoming edge.
    #[display("input port '{port}' on node '{node}' has no incoming edge")]
    DisconnectedInput {
        /// The node name.
        node: String,
        /// The port name.
        port: String,
    },
    /// The graph has no nodes.
    #[display("graph has no nodes")]
    EmptyGraph,
    /// Multiple edges connect to the same input port.
    #[display(
        "duplicate connection to input port '{target_port}' on node '{target_node}': \
         already connected from '{existing_source}', attempted from '{new_source}'"
    )]
    DuplicateConnection {
        /// The target node name.
        target_node: String,
        /// The target port name.
        target_port: String,
        /// The existing source node name.
        existing_source: String,
        /// The new source node name attempting to connect.
        new_source: String,
    },
}

/// Builder for constructing a [`WorkflowGraph`].
///
/// Accumulates nodes and edges, then validates on [`build`](Self::build).
pub struct WorkflowGraphBuilder {
    /// Registered nodes: (name, node).
    nodes: Vec<(String, Box<dyn WorkflowNode>)>,
    /// Pending edges: (src_name, src_port, tgt_name, tgt_port).
    edges: Vec<(String, String, String, String)>,
}

impl WorkflowGraphBuilder {
    /// Creates a new empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Adds a node to the graph.
    pub fn add_node(&mut self, name: String, node: Box<dyn WorkflowNode>) -> &mut Self {
        self.nodes.push((name, node));
        self
    }

    /// Connects a source node's output port to a target node's input port.
    ///
    /// Validates that both nodes exist, both ports exist on their respective nodes,
    /// and the port types match. Returns an error if validation fails.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::NodeNotFound`] if either node doesn't exist.
    /// Returns [`GraphError::PortNotFound`] if either port doesn't exist on its node.
    /// Returns [`GraphError::TypeMismatch`] if port types don't match.
    /// Returns [`GraphError::DuplicateConnection`] if the target port already has an incoming edge.
    pub fn connect(
        &mut self,
        source_node: &str,
        source_port: &str,
        target_node: &str,
        target_port: &str,
    ) -> Result<&mut Self, Report<GraphError>> {
        let (source_type, target_type) =
            self.validate_connection(source_node, source_port, target_node, target_port)?;

        // Check for duplicate connections to the same target port.
        for (existing_src, _, existing_tgt, existing_tgt_port) in &self.edges {
            if existing_tgt == target_node && existing_tgt_port == target_port {
                return Err(Report::new(GraphError::DuplicateConnection {
                    target_node: target_node.to_owned(),
                    target_port: target_port.to_owned(),
                    existing_source: existing_src.clone(),
                    new_source: source_node.to_owned(),
                }));
            }
        }

        // Type mismatch check.
        if source_type != target_type {
            return Err(Report::new(GraphError::TypeMismatch {
                source_node: source_node.to_owned(),
                source_port: source_port.to_owned(),
                source_type,
                target_node: target_node.to_owned(),
                target_port: target_port.to_owned(),
                target_type,
            }));
        }

        self.edges.push((
            source_node.to_owned(),
            source_port.to_owned(),
            target_node.to_owned(),
            target_port.to_owned(),
        ));
        Ok(self)
    }

    #[expect(
        clippy::missing_panics_doc,
        reason = "panic is only possible if connect() validation is wrong"
    )]
    /// Builds the graph, performing final validation.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::EmptyGraph`] if no nodes were added.
    /// Returns [`GraphError::CycleDetected`] if the graph has a cycle.
    /// Returns [`GraphError::DisconnectedInput`] if any input port has no incoming edge.
    pub fn build(self) -> Result<WorkflowGraph, Report<GraphError>> {
        if self.nodes.is_empty() {
            return Err(Report::new(GraphError::EmptyGraph));
        }

        let mut graph = DiGraph::new();
        let mut name_to_index = HashMap::new();

        // Add nodes to petgraph.
        for (name, node) in self.nodes {
            let idx = graph.add_node(NodeData {
                name: name.clone(),
                node,
            });
            name_to_index.insert(name, idx);
        }

        // Build reverse map.
        let index_to_name: HashMap<_, _> =
            name_to_index.iter().map(|(n, &i)| (i, n.clone())).collect();

        // Add edges to petgraph (already validated by connect()).
        for (src_name, src_port, tgt_name, tgt_port) in &self.edges {
            #[expect(clippy::expect_used, reason = "validated during connect")]
            let src_idx = name_to_index
                .get(src_name)
                .copied()
                .expect("source node validated during connect");
            #[expect(clippy::expect_used, reason = "validated during connect")]
            let tgt_idx = name_to_index
                .get(tgt_name)
                .copied()
                .expect("target node validated during connect");

            // Look up port type from source node's output ports.
            let source_node = &graph[src_idx].node;
            #[expect(clippy::expect_used, reason = "validated during connect")]
            let port_type = find_port_type(&source_node.output_ports(), src_port)
                .expect("port validated during connect");

            graph.add_edge(
                src_idx,
                tgt_idx,
                EdgeData {
                    source_port: src_port.clone(),
                    target_port: tgt_port.clone(),
                    port_type,
                },
            );
        }

        // Cycle detection.
        if is_cyclic_directed(&graph) {
            return Err(Report::new(GraphError::CycleDetected));
        }

        // Check that every input port has at least one incoming edge.
        for (name, &idx) in &name_to_index {
            let node = &graph[idx].node;
            let input_ports = node.input_ports();

            for port_def in &input_ports {
                let has_edge = graph
                    .edges_directed(idx, petgraph::Direction::Incoming)
                    .any(|e| e.weight().target_port == port_def.name);

                if !has_edge {
                    return Err(Report::new(GraphError::DisconnectedInput {
                        node: name.clone(),
                        port: port_def.name.to_owned(),
                    }));
                }
            }
        }

        // Compute sources and sinks.
        let sources: Vec<String> = name_to_index
            .iter()
            .filter(|&(_, &idx)| {
                let node = &graph[idx].node;
                node.input_ports().is_empty()
            })
            .map(|(name, _)| name.clone())
            .collect();

        let sinks: Vec<String> = name_to_index
            .iter()
            .filter(|&(_, &idx)| {
                graph
                    .edges_directed(idx, petgraph::Direction::Outgoing)
                    .count()
                    == 0
            })
            .map(|(name, _)| name.clone())
            .collect();

        Ok(WorkflowGraph {
            inner: graph,
            name_to_index,
            index_to_name,
            sources,
            sinks,
        })
    }

    /// Validates a connection and returns the port types.
    fn validate_connection(
        &self,
        source_node: &str,
        source_port: &str,
        target_node: &str,
        target_port: &str,
    ) -> Result<(PortType, PortType), Report<GraphError>> {
        // Find source node.
        let source = self
            .nodes
            .iter()
            .find(|(n, _)| n == source_node)
            .ok_or_else(|| {
                Report::new(GraphError::NodeNotFound {
                    name: source_node.to_owned(),
                })
            })?;

        // Find target node.
        let target = self
            .nodes
            .iter()
            .find(|(n, _)| n == target_node)
            .ok_or_else(|| {
                Report::new(GraphError::NodeNotFound {
                    name: target_node.to_owned(),
                })
            })?;

        // Find source output port.
        let source_type =
            find_port_type(&source.1.output_ports(), source_port).ok_or_else(|| {
                Report::new(GraphError::PortNotFound {
                    node: source_node.to_owned(),
                    port: source_port.to_owned(),
                })
            })?;

        // Find target input port.
        let target_type =
            find_port_type(&target.1.input_ports(), target_port).ok_or_else(|| {
                Report::new(GraphError::PortNotFound {
                    node: target_node.to_owned(),
                    port: target_port.to_owned(),
                })
            })?;

        Ok((source_type, target_type))
    }
}

impl Default for WorkflowGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Finds the type of a named port in a list of port definitions.
fn find_port_type(ports: &[PortDef], name: &str) -> Option<PortType> {
    ports.iter().find(|p| p.name == name).map(|p| p.value_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeContext;
    use crate::port::{PortDef, PortValues};

    /// A simple test node with configurable ports.
    struct TestNode {
        node_name: &'static str,
        inputs: Vec<PortDef>,
        outputs: Vec<PortDef>,
    }

    impl TestNode {
        fn new(name: &'static str, inputs: Vec<PortDef>, outputs: Vec<PortDef>) -> Self {
            Self {
                node_name: name,
                inputs,
                outputs,
            }
        }

        fn source(name: &'static str) -> Self {
            Self::new(name, vec![], vec![PortDef::string("out")])
        }

        fn sink(name: &'static str) -> Self {
            Self::new(name, vec![PortDef::string("in")], vec![])
        }

        fn passthrough(name: &'static str) -> Self {
            Self::new(
                name,
                vec![PortDef::string("in")],
                vec![PortDef::string("out")],
            )
        }
    }

    #[expect(dead_code, reason = "used by engine in Phase 3")]
    struct TestContext;
    impl NodeContext for TestContext {}

    #[async_trait::async_trait]
    impl WorkflowNode for TestNode {
        fn name(&self) -> &'static str {
            self.node_name
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
        ) -> Result<PortValues, error_stack::Report<crate::node::NodeError>> {
            Ok(PortValues::new())
        }

        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(TestNode {
                node_name: self.node_name,
                inputs: self.inputs.clone(),
                outputs: self.outputs.clone(),
            })
        }
    }

    #[test]
    fn linear_graph_builds_successfully() {
        // Given three nodes: A → B → C.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::passthrough("b")))
            .add_node("c".to_owned(), Box::new(TestNode::sink("c")));

        builder.connect("a", "out", "b", "in").expect("connect a→b");
        builder.connect("b", "out", "c", "in").expect("connect b→c");

        // When building.
        let graph = builder.build().expect("build");

        // Then the graph has 3 nodes, 2 edges.
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn fan_out_graph_builds_successfully() {
        // Given: A → B and A → C.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::sink("b")))
            .add_node("c".to_owned(), Box::new(TestNode::sink("c")));

        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("a", "out", "c", "in").expect("a→c");

        // When building.
        let graph = builder.build().expect("build");

        // Then it succeeds with 3 nodes, 2 edges.
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn fan_in_graph_builds_successfully() {
        // Given: A → C and B → C, where C has two input ports.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::source("b")))
            .add_node(
                "c".to_owned(),
                Box::new(TestNode::new(
                    "c",
                    vec![PortDef::string("left"), PortDef::string("right")],
                    vec![PortDef::string("out")],
                )),
            );

        builder.connect("a", "out", "c", "left").expect("a→c");
        builder.connect("b", "out", "c", "right").expect("b→c");

        // When building.
        let graph = builder.build().expect("build");

        // Then it succeeds.
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn connect_rejects_unknown_source_node() {
        // Given a builder with one node.
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node("a".to_owned(), Box::new(TestNode::source("a")));

        // When connecting from a non-existent node.
        let result = builder.connect("unknown", "out", "a", "in");

        // Then it returns NodeNotFound.
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), GraphError::NodeNotFound { name } if name == "unknown")
        ));
    }

    #[test]
    fn connect_rejects_unknown_target_node() {
        // Given a builder with one node.
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node("a".to_owned(), Box::new(TestNode::source("a")));

        // When connecting to a non-existent node.
        let result = builder.connect("a", "out", "unknown", "in");

        // Then it returns NodeNotFound.
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), GraphError::NodeNotFound { name } if name == "unknown")
        ));
    }

    #[test]
    fn connect_rejects_unknown_source_port() {
        // Given a builder with two nodes.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::sink("b")));

        // When connecting from a non-existent port.
        let result = builder.connect("a", "nonexistent", "b", "in");

        // Then it returns PortNotFound.
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), GraphError::PortNotFound { node, port } if node == "a" && port == "nonexistent")
        ));
    }

    #[test]
    fn connect_rejects_unknown_target_port() {
        // Given a builder with two nodes.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::sink("b")));

        // When connecting to a non-existent port.
        let result = builder.connect("a", "out", "b", "nonexistent");

        // Then it returns PortNotFound.
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), GraphError::PortNotFound { node, port } if node == "b" && port == "nonexistent")
        ));
    }

    #[test]
    fn connect_rejects_type_mismatch() {
        // Given a node with String output and a node with Json input.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node(
                "b".to_owned(),
                Box::new(TestNode::new("b", vec![PortDef::json("in")], vec![])),
            );

        // When connecting String output to Json input.
        let result = builder.connect("a", "out", "b", "in");

        // Then it returns TypeMismatch.
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), GraphError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn build_rejects_cyclic_graph() {
        // Given a cycle: A → B → A.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::passthrough("a")))
            .add_node("b".to_owned(), Box::new(TestNode::passthrough("b")));

        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("b", "out", "a", "in").expect("b→a");

        // When building.
        let result = builder.build();

        // Then it returns CycleDetected.
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), GraphError::CycleDetected)
        ));
    }

    #[test]
    fn build_rejects_disconnected_input_port() {
        // Given a node B whose "in" port has no incoming edge.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::passthrough("b")));

        // No edges at all.

        // When building.
        let result = builder.build();

        // Then it returns DisconnectedInput for B's "in" port.
        assert!(matches!(
            result,
            Err(e) if matches!(
                e.current_context(),
                GraphError::DisconnectedInput { node, port } if node == "b" && port == "in"
            )
        ));
    }

    #[test]
    fn build_rejects_empty_graph() {
        // Given an empty builder.
        let builder = WorkflowGraphBuilder::new();

        // When building.
        let result = builder.build();

        // Then it returns EmptyGraph.
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), GraphError::EmptyGraph)
        ));
    }

    #[test]
    fn build_rejects_duplicate_connection_to_same_target_port() {
        // Given two sources and one target with one input port.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::source("b")))
            .add_node("c".to_owned(), Box::new(TestNode::sink("c")));

        builder.connect("a", "out", "c", "in").expect("a→c");

        // When connecting B to the same target port.
        let result = builder.connect("b", "out", "c", "in");

        // Then it returns DuplicateConnection.
        assert!(matches!(
            result,
            Err(e) if matches!(e.current_context(), GraphError::DuplicateConnection { target_node, target_port, .. }
                if target_node == "c" && target_port == "in")
        ));
    }

    #[test]
    fn sources_and_sinks_computed_correctly() {
        // Given A → B → C (A is source, C is sink).
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::passthrough("b")))
            .add_node("c".to_owned(), Box::new(TestNode::sink("c")));

        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("b", "out", "c", "in").expect("b→c");

        // When building.
        let graph = builder.build().expect("build");

        // Then sources = [a], sinks = [c].
        assert_eq!(graph.sources(), &["a"]);
        assert_eq!(graph.sinks(), &["c"]);
    }

    #[test]
    fn graph_with_only_source_nodes_builds() {
        // Given two source nodes with no edges.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::source("b")));

        // When building.
        let graph = builder.build().expect("build");

        // Then both are sources and both are sinks.
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.sources().len(), 2);
        assert_eq!(graph.sinks().len(), 2);
    }
}
