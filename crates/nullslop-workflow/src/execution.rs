//! Workflow execution state types.
//!
//! Provides types for monitoring workflow execution:
//! - [`WorkflowStructure`] — lightweight, Clone-able workflow topology
//! - [`OwnedEdgeInfo`] — owned edge description
//! - [`NodePorts`] — port definitions for a single node
//! - [`ExecutionSnapshot`] — immutable point-in-time snapshot of execution state
//! - [`WorkflowExecution`] — manages execution state with atomic snapshots

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::engine::NodeStatus;
use crate::graph::WorkflowGraph;
use crate::port::{PortDef, PortValues};
use crate::port::PortType;

/// Owned version of [`EdgeInfo`](crate::graph::EdgeInfo).
///
/// Describes a connection from one node's output port to another's input port.
/// Unlike [`EdgeInfo`](crate::graph::EdgeInfo), this type owns its strings and
/// has no lifetime dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedEdgeInfo {
    /// Source node name.
    pub source_node: String,
    /// Source port name.
    pub source_port: String,
    /// Target node name.
    pub target_node: String,
    /// Target port name.
    pub target_port: String,
    /// The port type flowing through this edge.
    pub port_type: PortType,
}

/// Port definitions for a single node.
///
/// Stores the declared input and output ports, suitable for rendering
/// and UI queries without needing the actual node implementation.
#[derive(Debug, Clone)]
pub struct NodePorts {
    /// Input port definitions.
    pub input_ports: Vec<PortDef>,
    /// Output port definitions.
    pub output_ports: Vec<PortDef>,
}

/// Lightweight, Clone-able workflow topology.
///
/// Contains node names, port definitions, and edge connections.
/// No node implementations (`Box<dyn WorkflowNode>`), no petgraph internals.
/// Suitable for rendering and UI queries.
///
/// Derived from [`WorkflowGraph`] via [`WorkflowGraph::extract_structure`].
#[derive(Debug, Clone)]
pub struct WorkflowStructure {
    /// Per-node port definitions, keyed by node name.
    node_ports: HashMap<String, NodePorts>,
    /// All edges in the graph.
    edges: Vec<OwnedEdgeInfo>,
    /// Source node names (entry points with no incoming edges).
    sources: Vec<String>,
    /// Sink node names (exit points with no outgoing edges).
    sinks: Vec<String>,
}

impl WorkflowStructure {
    /// Creates a new workflow structure from its components.
    pub(crate) fn new(
        node_ports: HashMap<String, NodePorts>,
        edges: Vec<OwnedEdgeInfo>,
        sources: Vec<String>,
        sinks: Vec<String>,
    ) -> Self {
        Self {
            node_ports,
            edges,
            sources,
            sinks,
        }
    }

    /// Returns an iterator over all node names in the graph.
    pub fn node_names(&self) -> impl Iterator<Item = &str> {
        self.node_ports.keys().map(String::as_str)
    }

    /// Returns the input port definitions for a named node.
    ///
    /// Returns `None` if no node with the given name exists.
    #[must_use]
    pub fn node_input_ports(&self, name: &str) -> Option<&[PortDef]> {
        self.node_ports.get(name).map(|np| np.input_ports.as_slice())
    }

    /// Returns the output port definitions for a named node.
    ///
    /// Returns `None` if no node with the given name exists.
    #[must_use]
    pub fn node_output_ports(&self, name: &str) -> Option<&[PortDef]> {
        self.node_ports.get(name).map(|np| np.output_ports.as_slice())
    }

    /// Returns all edges in the graph.
    pub fn edges(&self) -> &[OwnedEdgeInfo] {
        &self.edges
    }

    /// Returns the names of source nodes (entry points).
    pub fn sources(&self) -> &[String] {
        &self.sources
    }

    /// Returns the names of sink nodes (exit points).
    pub fn sinks(&self) -> &[String] {
        &self.sinks
    }

    /// Returns the number of nodes in the graph.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.node_ports.len()
    }

    /// Returns the number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// Per-node execution state captured in an [`ExecutionSnapshot`].
///
/// Combines status, static configuration, and runtime I/O data.
/// Values are stored behind `Arc` so snapshot cloning is cheap (refcount bumps).
#[derive(Debug, Clone)]
pub struct NodeState {
    /// Current execution status of this node.
    pub status: NodeStatus,
    /// Static node configuration, captured once at construction.
    /// `None` for nodes that don't override [`WorkflowNode::config()`](crate::node::WorkflowNode::config).
    pub config: Option<Arc<serde_json::Value>>,
    /// Input values fed to this node, captured before execution.
    /// `None` until the engine spawns the node.
    pub inputs: Option<Arc<PortValues>>,
    /// Output values produced by this node, captured after successful execution.
    /// `None` until the node completes.
    pub outputs: Option<Arc<PortValues>>,
}

/// An immutable snapshot of workflow execution state at a point in time.
///
/// Contains the topology (never changes) and per-node state (status, config, I/O).
/// Obtained via [`WorkflowExecution::snapshot()`]. Cheap to hold —
/// just an `Arc` reference count increment.
#[derive(Debug)]
pub struct ExecutionSnapshot {
    /// The workflow topology.
    structure: Arc<WorkflowStructure>,
    /// Per-node execution state.
    node_states: HashMap<String, NodeState>,
}

impl ExecutionSnapshot {
    /// Returns the workflow topology.
    #[must_use]
    pub fn structure(&self) -> &WorkflowStructure {
        &self.structure
    }

    /// Returns the status of a node by name.
    #[must_use]
    pub fn status_of(&self, node_name: &str) -> Option<NodeStatus> {
        self.node_states.get(node_name).map(|s| s.status)
    }

    /// Returns an iterator over all node statuses.
    pub fn statuses(&self) -> impl Iterator<Item = (&str, NodeStatus)> {
        self.node_states.iter().map(|(k, v)| (k.as_str(), v.status))
    }

    /// Returns the full state for a node by name.
    #[must_use]
    pub fn node_state(&self, node_name: &str) -> Option<&NodeState> {
        self.node_states.get(node_name)
    }

    /// Returns an iterator over all node states.
    pub fn node_states(&self) -> impl Iterator<Item = (&str, &NodeState)> {
        self.node_states.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// Manages the execution state of a workflow graph.
///
/// Created from a [`WorkflowGraph`]. The engine writes status updates into
/// this type during execution. Consumers read from it via [`snapshot()`](Self::snapshot).
///
/// Internally uses [`ArcSwap`] for lock-free atomic snapshots.
pub struct WorkflowExecution {
    graph: WorkflowGraph,
    /// Lock-free atomic snapshot — swapped on every status update.
    snapshot: ArcSwap<ExecutionSnapshot>,
}

impl std::fmt::Debug for WorkflowExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowExecution")
            .field("node_count", &self.graph.node_count())
            .field("snapshot", &self.snapshot.load())
            .finish()
    }
}

impl WorkflowExecution {
    /// Creates a new execution from a built graph.
    ///
    /// All nodes are initialized to [`NodeStatus::Pending`].
    /// The structure is derived from the graph immediately.
    pub fn new(graph: WorkflowGraph) -> Self {
        let structure = Arc::new(extract_structure(&graph));
        let node_states = structure
            .node_names()
            .map(|name| {
                (
                    name.to_owned(),
                    NodeState {
                        status: NodeStatus::Pending,
                        config: None,
                        inputs: None,
                        outputs: None,
                    },
                )
            })
            .collect();
        let snapshot = ExecutionSnapshot { structure, node_states };
        Self {
            graph,
            snapshot: ArcSwap::from_pointee(snapshot),
        }
    }

    /// Returns the current execution snapshot.
    ///
    /// Lock-free atomic read. The returned `Arc` is immutable and
    /// won't change — safe to hold across an entire render frame.
    #[must_use]
    pub fn snapshot(&self) -> Arc<ExecutionSnapshot> {
        self.snapshot.load_full()
    }

    /// Returns a reference to the underlying graph for the engine.
    pub(crate) fn graph(&self) -> &WorkflowGraph {
        &self.graph
    }

    /// Updates a node's status. Atomically swaps in a new snapshot.
    ///
    /// Called by the engine on state transitions. Creates a new
    /// [`ExecutionSnapshot`] with the updated status map and swaps it in.
    /// The previous snapshot stays alive as long as any reader holds an `Arc`.
    pub fn set_status(&self, node_name: &str, status: NodeStatus) {
        let current = self.snapshot.load();
        let mut new_states = current.node_states.clone();
        if let Some(state) = new_states.get_mut(node_name) {
            state.status = status;
        }
        let new_snapshot = ExecutionSnapshot {
            structure: Arc::clone(&current.structure),
            node_states: new_states,
        };
        self.snapshot.store(Arc::new(new_snapshot));
    }
}

/// Extracts a lightweight [`WorkflowStructure`] from a [`WorkflowGraph`].
///
/// Uses only the public API of `WorkflowGraph`. The graph remains intact
/// for the engine to use.
fn extract_structure(graph: &WorkflowGraph) -> WorkflowStructure {
    let mut node_ports = HashMap::new();
    for name in graph.node_names() {
        let input_ports = graph.node_input_ports(name).unwrap_or_default();
        let output_ports = graph.node_output_ports(name).unwrap_or_default();
        node_ports.insert(
            name.to_owned(),
            NodePorts {
                input_ports,
                output_ports,
            },
        );
    }

    let edges: Vec<OwnedEdgeInfo> = graph
        .edges()
        .map(|edge| OwnedEdgeInfo {
            source_node: edge.source_node.to_owned(),
            source_port: edge.source_port.to_owned(),
            target_node: edge.target_node.to_owned(),
            target_port: edge.target_port.to_owned(),
            port_type: edge.port_type,
        })
        .collect();

    let sources = graph.sources().to_vec();
    let sinks = graph.sinks().to_vec();

    WorkflowStructure::new(node_ports, edges, sources, sinks)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;
    use crate::graph::WorkflowGraphBuilder;
    use crate::node::code::CodeNode;
    use crate::port::{PortDef, PortValue};

    /// Helper: builds a linear A → B → C graph.
    fn linear_graph() -> crate::graph::WorkflowGraph {
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node(
            "a".to_owned(),
            Box::new(CodeNode::new(
                "a".to_owned(),
                vec![],
                vec![PortDef::string("out")],
                |_inputs, _ctx| Box::pin(async { Ok(crate::port::PortValues::new()) }),
            )),
        );
        builder.add_node(
            "b".to_owned(),
            Box::new(CodeNode::new(
                "b".to_owned(),
                vec![PortDef::string("in")],
                vec![PortDef::string("out")],
                |mut inputs, _ctx| {
                    Box::pin(async move {
                        let val = inputs
                            .take_string("in")
                            .map_err(|_e| error_stack::Report::new(crate::node::NodeError))?;
                        let mut out = crate::port::PortValues::new();
                        out.insert("out".to_owned(), PortValue::String(val));
                        Ok(out)
                    })
                },
            )),
        );
        builder.add_node(
            "c".to_owned(),
            Box::new(CodeNode::new(
                "c".to_owned(),
                vec![PortDef::string("in")],
                vec![],
                |_inputs, _ctx| Box::pin(async { Ok(crate::port::PortValues::new()) }),
            )),
        );
        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("b", "out", "c", "in").expect("b→c");
        builder.build().expect("linear graph")
    }

    #[rstest::rstest]
    fn structure_matches_linear_graph() {
        // Given a linear A → B → C graph.
        let graph = linear_graph();

        // When extracting the structure.
        let structure = extract_structure(&graph);

        // Then all nodes are present.
        let names: Vec<&str> = structure.node_names().collect();
        assert_eq!(names.len(), 3);

        // And edges are correct.
        assert_eq!(structure.edge_count(), 2);

        // And sources and sinks are correct.
        assert_eq!(structure.sources(), &["a".to_owned()]);
        assert_eq!(structure.sinks(), &["c".to_owned()]);

        // And node ports are correct.
        assert_eq!(structure.node_input_ports("a"), Some(&[][..]));
        assert_eq!(structure.node_output_ports("a"), Some(&[PortDef::string("out")][..]));
        assert_eq!(structure.node_input_ports("b"), Some(&[PortDef::string("in")][..]));
        assert_eq!(structure.node_output_ports("b"), Some(&[PortDef::string("out")][..]));
    }

    #[rstest::rstest]
    fn structure_is_cloneable() {
        // Given a structure from a linear graph.
        let graph = linear_graph();
        let structure = extract_structure(&graph);

        // When cloning.
        let cloned = structure.clone();

        // Then the clone has the same data.
        assert_eq!(cloned.node_count(), structure.node_count());
        assert_eq!(cloned.edge_count(), structure.edge_count());
    }

    #[rstest::rstest]
    fn execution_initial_state_is_all_pending() {
        // Given a new execution from a linear graph.
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);

        // When taking a snapshot.
        let snapshot = execution.snapshot();

        // Then all nodes are Pending.
        for name in snapshot.structure().node_names() {
            assert_eq!(snapshot.status_of(name), Some(NodeStatus::Pending));
        }
    }

    #[rstest::rstest]
    fn set_status_updates_current_snapshot() {
        // Given a new execution.
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);

        // When setting a node to Running.
        execution.set_status("a", NodeStatus::Running);

        // Then the current snapshot reflects the change.
        let snapshot = execution.snapshot();
        assert_eq!(snapshot.status_of("a"), Some(NodeStatus::Running));
        assert_eq!(snapshot.status_of("b"), Some(NodeStatus::Pending));
    }

    #[rstest::rstest]
    fn snapshot_is_isolated_from_later_updates() {
        // Given a new execution.
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);

        // When taking a snapshot before updates.
        let s1 = execution.snapshot();
        assert_eq!(s1.status_of("a"), Some(NodeStatus::Pending));

        // And then updating statuses.
        execution.set_status("a", NodeStatus::Running);
        execution.set_status("b", NodeStatus::Completed);
        execution.set_status("a", NodeStatus::Completed);

        // Then the first snapshot is unchanged.
        assert_eq!(s1.status_of("a"), Some(NodeStatus::Pending));
        assert_eq!(s1.status_of("b"), Some(NodeStatus::Pending));
        assert_eq!(s1.status_of("c"), Some(NodeStatus::Pending));

        // And the current snapshot has the updates.
        let s2 = execution.snapshot();
        assert_eq!(s2.status_of("a"), Some(NodeStatus::Completed));
        assert_eq!(s2.status_of("b"), Some(NodeStatus::Completed));
        assert_eq!(s2.status_of("c"), Some(NodeStatus::Pending));
    }

    #[rstest::rstest]
    fn multiple_snapshots_from_different_times() {
        // Given a new execution.
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);

        // When taking snapshots at different times.
        let s1 = execution.snapshot(); // all Pending
        execution.set_status("a", NodeStatus::Running);
        let s2 = execution.snapshot();
        execution.set_status("b", NodeStatus::Running);
        let s3 = execution.snapshot();

        // Then each snapshot reflects state at its time.
        assert_eq!(s1.status_of("a"), Some(NodeStatus::Pending));
        assert_eq!(s2.status_of("a"), Some(NodeStatus::Running));
        assert_eq!(s2.status_of("b"), Some(NodeStatus::Pending));
        assert_eq!(s3.status_of("b"), Some(NodeStatus::Running));
    }

    #[tokio::test]
    async fn snapshot_held_across_concurrent_updates() {
        // Given a new execution.
        let graph = linear_graph();
        let execution = Arc::new(WorkflowExecution::new(graph));

        // When taking an early snapshot.
        let s1 = execution.snapshot();

        // And then cycling all nodes through Running → Completed in another task.
        let exec_clone = execution.clone();
        tokio::spawn(async move {
            exec_clone.set_status("a", NodeStatus::Running);
            exec_clone.set_status("b", NodeStatus::Running);
            exec_clone.set_status("c", NodeStatus::Running);
            exec_clone.set_status("a", NodeStatus::Completed);
            exec_clone.set_status("b", NodeStatus::Completed);
            exec_clone.set_status("c", NodeStatus::Completed);
        })
        .await
        .expect("task should complete");

        // Then the early snapshot is still all Pending.
        assert_eq!(s1.status_of("a"), Some(NodeStatus::Pending));
        assert_eq!(s1.status_of("b"), Some(NodeStatus::Pending));
        assert_eq!(s1.status_of("c"), Some(NodeStatus::Pending));

        // And the current snapshot shows all Completed.
        let current = execution.snapshot();
        assert_eq!(current.status_of("a"), Some(NodeStatus::Completed));
        assert_eq!(current.status_of("b"), Some(NodeStatus::Completed));
        assert_eq!(current.status_of("c"), Some(NodeStatus::Completed));
    }
}
