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
use crate::validation::{ValidationDiagnostic, ValidationSeverity};

/// Internal storage for a node in the graph.
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "pub(crate) on pub(crate) struct keeps fields crate-local"
)]
pub(crate) struct NodeData {
    /// Node name.
    #[expect(dead_code, reason = "used for graph introspection")]
    name: String,
    /// The workflow node implementation.
    pub(crate) node: Box<dyn WorkflowNode>,
}

/// Internal storage for an edge in the graph.
#[derive(Debug)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "pub(crate) on pub(crate) struct keeps fields crate-local"
)]
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
    /// Optional human-readable description of the workflow.
    description: Option<String>,
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

    /// Returns the config value for a named node.
    ///
    /// Returns `None` if the node doesn't exist or doesn't override `config()`.
    pub(crate) fn node_config(&self, name: &str) -> Option<serde_json::Value> {
        let idx = self.name_to_index.get(name)?;
        self.inner[*idx].node.config()
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

    /// Validates the graph and returns diagnostics for potential issues.
    ///
    /// Unlike [`build`](WorkflowGraphBuilder::build), which checks hard requirements
    /// and returns errors, `validate` returns warnings for situations that are
    /// technically valid but may indicate a mistake.
    ///
    /// Current checks:
    ///
    /// - **Isolated nodes** - nodes with no incoming or outgoing edges.
    /// - **Dead output ports** - output ports with no outgoing edge.
    #[must_use]
    pub fn validate(&self) -> Vec<ValidationDiagnostic> {
        let mut diagnostics = Vec::new();

        for (name, &idx) in &self.name_to_index {
            let node = &self.inner[idx].node;

            let has_incoming = self
                .inner
                .edges_directed(idx, petgraph::Direction::Incoming)
                .count()
                > 0;
            let has_outgoing = self
                .inner
                .edges_directed(idx, petgraph::Direction::Outgoing)
                .count()
                > 0;

            if !has_incoming && !has_outgoing {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Warning,
                    message: format!("node '{name}' is isolated (no incoming or outgoing edges)"),
                });
            }

            for port in node.output_ports() {
                let has_edge = self
                    .inner
                    .edges_directed(idx, petgraph::Direction::Outgoing)
                    .any(|e| e.weight().source_port == port.name);

                if !has_edge {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Warning,
                        message: format!(
                            "output port '{}' on node '{}' has no outgoing edge",
                            port.name, name
                        ),
                    });
                }
            }
        }

        diagnostics
    }

    /// Returns the human-readable description of this workflow, if set.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
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
    /// An internal invariant was violated during graph building.
    /// These should all be unreachable after the connect-time validation.
    #[display("internal invariant violated during build: {what}")]
    InternalInvariant {
        /// Description of the invariant that failed.
        what: &'static str,
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
    /// Optional human-readable description.
    description: Option<String>,
}

impl WorkflowGraphBuilder {
    /// Creates a new empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            description: None,
        }
    }

    /// Sets a human-readable description for the workflow.
    #[must_use]
    pub fn with_description<S>(mut self, desc: S) -> Self
    where
        S: Into<String>,
    {
        self.description = Some(desc.into());
        self
    }

    /// Adds a node to the graph.
    pub fn add_node(&mut self, name: String, node: Box<dyn WorkflowNode>) -> &mut Self {
        self.nodes.push((name, node));
        self
    }

    /// Adds a node from the registry to the graph.
    ///
    /// Looks up the factory for `type_name` in the registry, creates a node
    /// with the given `config`, and adds it to the graph with the given `name`.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::NotFound`] if the type name is not registered.
    /// Returns [`RegistryError::CreationFailed`] if the factory fails.
    pub fn add_node_from_registry(
        &mut self,
        name: String,
        registry: &crate::registry::NodeRegistry,
        type_name: &str,
        config: serde_json::Value,
    ) -> Result<&mut Self, error_stack::Report<crate::registry::RegistryError>> {
        let node = registry.create(type_name, config)?;
        self.add_node(name, node);
        Ok(self)
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
            let src_idx = name_to_index.get(src_name).copied().ok_or_else(|| {
                Report::new(GraphError::InternalInvariant {
                    what: "source node validated during connect",
                })
            })?;
            let tgt_idx = name_to_index.get(tgt_name).copied().ok_or_else(|| {
                Report::new(GraphError::InternalInvariant {
                    what: "target node validated during connect",
                })
            })?;

            // Look up port type from source node's output ports.
            let source_node = &graph[src_idx].node;
            let port_type =
                find_port_type(&source_node.output_ports(), src_port).ok_or_else(|| {
                    Report::new(GraphError::InternalInvariant {
                        what: "port validated during connect",
                    })
                })?;

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

                if !port_def.required && !has_edge {
                    // Optional port - allowed to be disconnected.
                    continue;
                }

                if !has_edge {
                    return Err(Report::new(GraphError::DisconnectedInput {
                        node: name.clone(),
                        port: port_def.name.clone(),
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
            description: self.description,
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
#[expect(clippy::expect_used, reason = "test assertions")]
mod tests {
    #![allow(clippy::unnecessary_literal_bound, reason = "test code")]
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
            Self::new(name, vec![], vec![PortDef::text("out")])
        }

        fn sink(name: &'static str) -> Self {
            Self::new(name, vec![PortDef::text("in")], vec![])
        }

        fn passthrough(name: &'static str) -> Self {
            Self::new(name, vec![PortDef::text("in")], vec![PortDef::text("out")])
        }
    }

    #[expect(dead_code, reason = "used by engine in Phase 3")]
    struct TestContext;
    impl NodeContext for TestContext {}

    #[async_trait::async_trait]
    impl WorkflowNode for TestNode {
        fn name(&self) -> &str {
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
                    vec![PortDef::text("left"), PortDef::text("right")],
                    vec![PortDef::text("out")],
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
        // Given a node with Text output and a node with Json input.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node(
                "b".to_owned(),
                Box::new(TestNode::new("b", vec![PortDef::json("in")], vec![])),
            );

        // When connecting Text output to Json input.
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

    #[test]
    fn optional_input_port_builds_without_connection() {
        // Given a node with one required and one optional input port.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node(
                "a".to_owned(),
                Box::new(TestNode::new(
                    "a",
                    vec![
                        PortDef::text("required"),
                        PortDef::text("optional").optional(),
                    ],
                    vec![PortDef::text("out")],
                )),
            )
            .add_node("b".to_owned(), Box::new(TestNode::source("b")));

        // Only connect the required port - optional port left disconnected.
        builder.connect("b", "out", "a", "required").expect("b→a");

        // When building.
        let graph = builder
            .build()
            .expect("build should succeed with optional port disconnected");

        // Then the graph builds with 2 nodes, 1 edge.
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn optional_input_port_rejects_disconnected_required_port() {
        // Given a node with one required and one optional input port.
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node(
            "a".to_owned(),
            Box::new(TestNode::new(
                "a",
                vec![
                    PortDef::text("required"),
                    PortDef::text("optional").optional(),
                ],
                vec![PortDef::text("out")],
            )),
        );

        // Connect nothing - required port is disconnected.
        // When building.
        let result = builder.build();

        // Then it returns DisconnectedInput for the required port.
        assert!(matches!(
            result,
            Err(e) if matches!(
                e.current_context(),
                GraphError::DisconnectedInput { node, port } if node == "a" && port == "required"
            )
        ));
    }

    #[test]
    fn optional_port_with_connection_builds_successfully() {
        // Given a node with one optional input port connected.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node(
                "a".to_owned(),
                Box::new(TestNode::new(
                    "a",
                    vec![PortDef::text("opt").optional()],
                    vec![PortDef::text("out")],
                )),
            )
            .add_node("b".to_owned(), Box::new(TestNode::source("b")));

        // Connect the optional port.
        builder.connect("b", "out", "a", "opt").expect("b→a");

        // When building.
        let graph = builder.build().expect("build");

        // Then it succeeds.
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn validate_warns_on_isolated_node() {
        // Given two source nodes with no edges (isolated).
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::source("b")));

        let graph = builder.build().expect("build");

        // When validating.
        let diagnostics = graph.validate();

        // Then each isolated node gets a warning.
        let isolated: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.message.contains("isolated"))
            .collect();
        assert_eq!(
            isolated.len(),
            2,
            "expected 2 isolated warnings, got: {diagnostics:?}"
        );
    }

    #[test]
    fn validate_warns_on_dead_output_port() {
        // Given A → B where A has an output port "out" but B is a sink.
        // A's output is connected, but B has no outputs - that's fine.
        // But if we have a node with an unconnected output...
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node(
                "a".to_owned(),
                Box::new(TestNode::new(
                    "a",
                    vec![],
                    vec![PortDef::text("out"), PortDef::text("extra")],
                )),
            )
            .add_node("b".to_owned(), Box::new(TestNode::sink("b")));

        // Only connect "out", leave "extra" unconnected.
        builder.connect("a", "out", "b", "in").expect("a→b");

        let graph = builder.build().expect("build");

        // When validating.
        let diagnostics = graph.validate();

        // Then "extra" port on node "a" gets a dead output warning.
        let dead: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.message.contains("'extra'") && d.message.contains("no outgoing edge"))
            .collect();
        assert_eq!(
            dead.len(),
            1,
            "expected 1 dead output warning, got: {diagnostics:?}"
        );
    }

    #[test]
    fn validate_passes_fully_connected_graph() {
        // Given A → B → C (fully connected linear graph).
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::passthrough("b")))
            .add_node("c".to_owned(), Box::new(TestNode::sink("c")));

        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("b", "out", "c", "in").expect("b→c");

        let graph = builder.build().expect("build");

        // When validating.
        let diagnostics = graph.validate();

        // Then no warnings.
        assert!(
            diagnostics.is_empty(),
            "expected no warnings, got: {diagnostics:?}"
        );
    }

    #[test]
    fn validate_returns_multiple_diagnostics() {
        // Given two isolated source nodes.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::source("b")));

        let graph = builder.build().expect("build");

        // When validating.
        let diagnostics = graph.validate();

        // Then we get multiple diagnostics (one isolated warning per node, plus one dead output per node).
        assert!(
            diagnostics.len() >= 2,
            "expected multiple diagnostics, got: {diagnostics:?}"
        );
    }

    #[test]
    fn graph_with_description_returns_description() {
        // Given a builder with a description.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::sink("b")));
        builder.connect("a", "out", "b", "in").expect("a\u{2192}b");

        // When building with description.
        let graph = builder
            .with_description("A test workflow")
            .build()
            .expect("build");

        // Then the description is set.
        assert_eq!(graph.description(), Some("A test workflow"));
    }

    #[test]
    fn graph_without_description_returns_none() {
        // Given a builder without a description.
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), Box::new(TestNode::source("a")))
            .add_node("b".to_owned(), Box::new(TestNode::sink("b")));
        builder.connect("a", "out", "b", "in").expect("a\u{2192}b");

        // When building without description.
        let graph = builder.build().expect("build");

        // Then the description is None.
        assert_eq!(graph.description(), None);
    }

    // --- Mutant-killing tests for graph.rs ---

    // Kills: node_config -> None (always)
    // TestNode doesn't override config(), so node_config returns None for it.
    // We need a node that overrides config() to test that node_config returns actual values.
    struct ConfigNode {
        config_val: serde_json::Value,
    }

    #[async_trait::async_trait]
    impl WorkflowNode for ConfigNode {
        fn name(&self) -> &str {
            "config_node"
        }
        fn input_ports(&self) -> Vec<PortDef> {
            vec![]
        }
        fn output_ports(&self) -> Vec<PortDef> {
            vec![PortDef::text("out")]
        }
        async fn execute(
            &self,
            _inputs: PortValues,
            _ctx: &dyn NodeContext,
        ) -> Result<PortValues, Report<crate::node::NodeError>> {
            Ok(PortValues::new())
        }
        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(ConfigNode {
                config_val: self.config_val.clone(),
            })
        }
        fn config(&self) -> Option<serde_json::Value> {
            Some(self.config_val.clone())
        }
    }

    #[test]
    fn node_config_returns_none_for_node_without_config() {
        // Kills: node_config -> Some(Default::default())
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node("a".to_owned(), Box::new(TestNode::source("a")));
        builder.add_node("b".to_owned(), Box::new(TestNode::sink("b")));
        builder.connect("a", "out", "b", "in").expect("a→b");
        let graph = builder.build().expect("build");
        // TestNode doesn't override config(), so this returns None.
        assert_eq!(
            graph.node_config("a"),
            None,
            "node without config must return None, not Some(default)"
        );
    }

    #[test]
    fn node_config_returns_actual_config_value() {
        // Kills: node_config -> None (always)
        let config = serde_json::json!({"prompt": "test", "temperature": 0.7});
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node(
            "cfg".to_owned(),
            Box::new(ConfigNode {
                config_val: config.clone(),
            }),
        );
        builder.add_node("b".to_owned(), Box::new(TestNode::sink("b")));
        builder.connect("cfg", "out", "b", "in").expect("cfg→b");
        let graph = builder.build().expect("build");
        let result = graph.node_config("cfg");
        assert_eq!(
            result,
            Some(config),
            "must return actual config, not None or Some(default)"
        );
    }
}
