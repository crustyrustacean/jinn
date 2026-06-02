//! Workflow execution state types.
//!
//! Provides types for monitoring workflow execution:
//! - [`WorkflowStructure`] - lightweight, Clone-able workflow topology
//! - [`OwnedEdgeInfo`] - owned edge description
//! - [`NodePorts`] - port definitions for a single node
//! - [`ExecutionSnapshot`] - immutable point-in-time snapshot of execution state
//! - [`WorkflowExecution`] - manages execution state with atomic snapshots

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::engine::NodeStatus;
use crate::graph::WorkflowGraph;
use crate::port::PortType;
use crate::port::{PortDef, PortValues};

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
        self.node_ports
            .get(name)
            .map(|np| np.input_ports.as_slice())
    }

    /// Returns the output port definitions for a named node.
    ///
    /// Returns `None` if no node with the given name exists.
    #[must_use]
    pub fn node_output_ports(&self, name: &str) -> Option<&[PortDef]> {
        self.node_ports
            .get(name)
            .map(|np| np.output_ports.as_slice())
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

    /// Returns child node names (direct downstream), deduplicated and sorted.
    ///
    /// Derived from edges where this node is the source.
    /// Used for graph-aware navigation and cascade invalidation.
    pub fn children_of(&self, node_name: &str) -> Vec<&str> {
        let mut children: Vec<&str> = self
            .edges
            .iter()
            .filter(|e| e.source_node == node_name)
            .map(|e| e.target_node.as_str())
            .collect();
        children.sort_unstable();
        children.dedup();
        children
    }

    /// Returns parent node names (direct upstream), deduplicated and sorted.
    ///
    /// Derived from edges where this node is the target.
    /// Used for graph-aware navigation and input seeding.
    pub fn parents_of(&self, node_name: &str) -> Vec<&str> {
        let mut parents: Vec<&str> = self
            .edges
            .iter()
            .filter(|e| e.target_node == node_name)
            .map(|e| e.source_node.as_str())
            .collect();
        parents.sort_unstable();
        parents.dedup();
        parents
    }

    /// Returns the set of all transitive downstream node names from the given node.
    ///
    /// Uses BFS over the edge list. Does not include the starting node itself.
    /// Used for cascade invalidation.
    pub fn downstream_of(&self, node_name: &str) -> HashSet<&str> {
        let mut downstream = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(node_name);
        while let Some(name) = queue.pop_front() {
            for edge in &self.edges {
                if edge.source_node == name && downstream.insert(edge.target_node.as_str()) {
                    queue.push_back(&edge.target_node);
                }
            }
        }
        downstream.remove(node_name);
        downstream
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
/// Obtained via [`WorkflowExecution::snapshot()`]. Cheap to hold -
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
    /// The graph topology (immutable after construction).
    graph: WorkflowGraph,
    /// Lock-free atomic snapshot - swapped on every status update.
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
                let config = graph.node_config(name).map(Arc::new);
                (
                    name.to_owned(),
                    NodeState {
                        status: NodeStatus::Pending,
                        config,
                        inputs: None,
                        outputs: None,
                    },
                )
            })
            .collect();
        let snapshot = ExecutionSnapshot {
            structure,
            node_states,
        };
        Self {
            graph,
            snapshot: ArcSwap::from_pointee(snapshot),
        }
    }

    /// Returns the current execution snapshot.
    ///
    /// Lock-free atomic read. The returned `Arc` is immutable and
    /// won't change - safe to hold across an entire render frame.
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

    /// Stores the inputs that will be fed to a node.
    ///
    /// Called by the engine before spawning the node task.
    /// Atomically swaps in a new snapshot with the updated inputs.
    pub fn set_node_inputs(&self, node_name: &str, inputs: PortValues) {
        let current = self.snapshot.load();
        let mut new_states = current.node_states.clone();
        if let Some(state) = new_states.get_mut(node_name) {
            state.inputs = Some(Arc::new(inputs));
        }
        self.snapshot.store(Arc::new(ExecutionSnapshot {
            structure: Arc::clone(&current.structure),
            node_states: new_states,
        }));
    }

    /// Stores the outputs produced by a completed node.
    ///
    /// Called by the engine on successful node completion.
    /// Atomically swaps in a new snapshot with the updated outputs.
    pub fn set_node_outputs(&self, node_name: &str, outputs: PortValues) {
        let current = self.snapshot.load();
        let mut new_states = current.node_states.clone();
        if let Some(state) = new_states.get_mut(node_name) {
            state.outputs = Some(Arc::new(outputs));
        }
        self.snapshot.store(Arc::new(ExecutionSnapshot {
            structure: Arc::clone(&current.structure),
            node_states: new_states,
        }));
    }

    /// Updates a node's cached output and invalidates all transitive downstream.
    ///
    /// Downstream nodes have their inputs, outputs cleared and status set to Pending.
    /// Does NOT trigger execution. The named node keeps its status unchanged.
    pub fn update_output(&self, node_name: &str, outputs: PortValues) {
        // 1. Update the node's own output.
        {
            let current = self.snapshot.load();
            let mut new_states = current.node_states.clone();
            if let Some(state) = new_states.get_mut(node_name) {
                state.outputs = Some(Arc::new(outputs));
            }
            self.snapshot.store(Arc::new(ExecutionSnapshot {
                structure: Arc::clone(&current.structure),
                node_states: new_states,
            }));
        }

        // 2. Invalidate downstream (exclude self).
        self.invalidate_from_inner(node_name, false);
    }

    /// Clears inputs, outputs, and sets status to Pending for the named node
    /// and all transitive downstream nodes.
    pub fn invalidate_from(&self, node_name: &str) {
        self.invalidate_from_inner(node_name, true);
    }

    /// Shared invalidation logic.
    ///
    /// `include_self`: when true, clears the named node too (for `invalidate_from`).
    /// When false, only downstream (for `update_output`).
    fn invalidate_from_inner(&self, node_name: &str, include_self: bool) {
        let current = self.snapshot.load();
        let downstream = current.structure.downstream_of(node_name);

        let mut new_states = current.node_states.clone();

        if include_self && let Some(state) = new_states.get_mut(node_name) {
            state.inputs = None;
            state.outputs = None;
            state.status = NodeStatus::Pending;
        }

        for name in &downstream {
            if let Some(state) = new_states.get_mut(*name) {
                state.inputs = None;
                state.outputs = None;
                state.status = NodeStatus::Pending;
            }
        }

        self.snapshot.store(Arc::new(ExecutionSnapshot {
            structure: Arc::clone(&current.structure),
            node_states: new_states,
        }));
    }

    /// Populates a node's inputs by reading upstream cached outputs
    /// and routing through the static edge definitions.
    ///
    /// Requires that parent nodes have cached outputs.
    /// Silently produces empty/partial inputs if parent outputs are missing.
    pub fn seed_inputs(&self, node_name: &str) {
        let current = self.snapshot.load();
        let structure = &current.structure;

        let mut inputs = PortValues::new();
        for edge in structure.edges() {
            if edge.target_node == node_name
                && let Some(parent_state) = current.node_states.get(&edge.source_node)
                && let Some(parent_outputs) = &parent_state.outputs
                && let Some(value) = parent_outputs.get(&edge.source_port)
            {
                inputs.insert(edge.target_port.clone(), value.clone());
            }
        }

        let mut new_states = current.node_states.clone();
        if let Some(state) = new_states.get_mut(node_name) {
            state.inputs = Some(Arc::new(inputs));
        }
        self.snapshot.store(Arc::new(ExecutionSnapshot {
            structure: Arc::clone(&current.structure),
            node_states: new_states,
        }));
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
    use crate::port::{PortDef, PortValue, ScalarValue};

    /// Helper: builds a linear A → B → C graph.
    fn linear_graph() -> crate::graph::WorkflowGraph {
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node(
            "a".to_owned(),
            Box::new(CodeNode::new(
                "a".to_owned(),
                vec![],
                vec![PortDef::text("out")],
                |_inputs, _ctx| Box::pin(async { Ok(crate::port::PortValues::new()) }),
            )),
        );
        builder.add_node(
            "b".to_owned(),
            Box::new(CodeNode::new(
                "b".to_owned(),
                vec![PortDef::text("in")],
                vec![PortDef::text("out")],
                |mut inputs, _ctx| {
                    Box::pin(async move {
                        let val = inputs
                            .take_text("in")
                            .map_err(|_e| error_stack::Report::new(crate::node::NodeError))?;
                        let mut out = crate::port::PortValues::new();
                        out.insert("out".to_owned(), PortValue::Single(ScalarValue::Text(val)));
                        Ok(out)
                    })
                },
            )),
        );
        builder.add_node(
            "c".to_owned(),
            Box::new(CodeNode::new(
                "c".to_owned(),
                vec![PortDef::text("in")],
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
        assert_eq!(
            structure.node_output_ports("a"),
            Some(&[PortDef::text("out")][..])
        );
        assert_eq!(
            structure.node_input_ports("b"),
            Some(&[PortDef::text("in")][..])
        );
        assert_eq!(
            structure.node_output_ports("b"),
            Some(&[PortDef::text("out")][..])
        );
    }

    // --- Mutant-killing tests for execution.rs ---

    #[rstest::rstest]
    fn node_count_returns_actual_node_count() {
        let graph = linear_graph();
        let structure = extract_structure(&graph);
        // linear_graph has 3 nodes. Kills: node_count -> 0, node_count -> 1
        assert_eq!(structure.node_count(), 3, "must return 3, not 0 or 1");
    }

    #[rstest::rstest]
    fn statuses_returns_non_empty_iterator() {
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let count = snapshot.statuses().count();
        // Kills: statuses -> iter::empty()
        assert_eq!(count, 3, "statuses() must yield all 3 nodes, not be empty");
    }

    #[rstest::rstest]
    fn node_states_returns_non_empty_iterator() {
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);
        let snapshot = execution.snapshot();
        let count = snapshot.node_states().count();
        // Kills: node_states -> iter::empty()
        assert_eq!(
            count, 3,
            "node_states() must yield all 3 nodes, not be empty"
        );
    }

    #[rstest::rstest]
    fn debug_impl_produces_non_empty_output() {
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);
        // Kills: Debug fmt -> Ok(Default::default())
        let debug_str = format!("{execution:?}");
        assert!(!debug_str.is_empty(), "Debug output must not be empty");
        assert!(
            debug_str.contains("node_count"),
            "Debug must contain node_count"
        );
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

    #[rstest::rstest]
    fn children_of_returns_direct_downstream() {
        let graph = linear_graph();
        let structure = extract_structure(&graph);

        assert_eq!(structure.children_of("a"), vec!["b"]);
        assert_eq!(structure.children_of("b"), vec!["c"]);
        assert_eq!(structure.children_of("c"), Vec::<&str>::new());
    }

    #[rstest::rstest]
    fn parents_of_returns_direct_upstream() {
        let graph = linear_graph();
        let structure = extract_structure(&graph);

        assert_eq!(structure.parents_of("a"), Vec::<&str>::new());
        assert_eq!(structure.parents_of("b"), vec!["a"]);
        assert_eq!(structure.parents_of("c"), vec!["b"]);
    }

    #[rstest::rstest]
    fn downstream_of_returns_transitive_closure() {
        let graph = linear_graph();
        let structure = extract_structure(&graph);

        let downstream = structure.downstream_of("a");
        assert!(downstream.contains("b"));
        assert!(downstream.contains("c"));
        assert!(!downstream.contains("a"));
        assert_eq!(downstream.len(), 2);

        let downstream_b = structure.downstream_of("b");
        assert!(downstream_b.contains("c"));
        assert_eq!(downstream_b.len(), 1);

        let downstream_c = structure.downstream_of("c");
        assert!(downstream_c.is_empty());
    }

    #[rstest::rstest]
    fn update_output_sets_output_and_invalidates_downstream() {
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);

        // Simulate: a and b completed, b has inputs.
        execution.set_status("a", NodeStatus::Completed);
        execution.set_status("b", NodeStatus::Completed);
        let mut a_out = PortValues::new();
        a_out.insert(
            "out".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        execution.set_node_outputs("a", a_out.clone());
        let mut b_out = PortValues::new();
        b_out.insert(
            "out".to_owned(),
            PortValue::Single(ScalarValue::Text("world".to_owned())),
        );
        execution.set_node_outputs("b", b_out.clone());
        execution.set_node_inputs("b", {
            let mut pv = PortValues::new();
            pv.insert(
                "in".to_owned(),
                PortValue::Single(ScalarValue::Text("hello".to_owned())),
            );
            pv
        });
        execution.set_node_inputs("c", {
            let mut pv = PortValues::new();
            pv.insert(
                "in".to_owned(),
                PortValue::Single(ScalarValue::Text("world".to_owned())),
            );
            pv
        });

        // Mutate a's output.
        let mut new_a_out = PortValues::new();
        new_a_out.insert(
            "out".to_owned(),
            PortValue::Single(ScalarValue::Text("changed".to_owned())),
        );
        execution.update_output("a", new_a_out.clone());

        let snap = execution.snapshot();

        // a's output changed but status stays Completed.
        assert_eq!(snap.status_of("a"), Some(NodeStatus::Completed));
        let a_state = snap.node_state("a").expect("a exists");
        assert_eq!(
            a_state
                .outputs
                .as_ref()
                .expect("has outputs")
                .get("out")
                .unwrap(),
            &PortValue::Single(ScalarValue::Text("changed".to_owned()))
        );

        // b is invalidated (cleared inputs/outputs, Pending).
        assert_eq!(snap.status_of("b"), Some(NodeStatus::Pending));
        let b_state = snap.node_state("b").expect("b exists");
        assert!(b_state.inputs.is_none());
        assert!(b_state.outputs.is_none());

        // c is also invalidated.
        assert_eq!(snap.status_of("c"), Some(NodeStatus::Pending));
        let c_state = snap.node_state("c").expect("c exists");
        assert!(c_state.inputs.is_none());
    }

    #[rstest::rstest]
    fn invalidate_from_clears_self_and_downstream() {
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);

        // Simulate: a completed with output.
        execution.set_status("a", NodeStatus::Completed);
        let mut a_out = PortValues::new();
        a_out.insert(
            "out".to_owned(),
            PortValue::Single(ScalarValue::Text("data".to_owned())),
        );
        execution.set_node_outputs("a", a_out);

        // Invalidate from a (includes a itself).
        execution.invalidate_from("a");

        let snap = execution.snapshot();
        assert_eq!(snap.status_of("a"), Some(NodeStatus::Pending));
        assert_eq!(snap.status_of("b"), Some(NodeStatus::Pending));
        assert_eq!(snap.status_of("c"), Some(NodeStatus::Pending));
        assert!(snap.node_state("a").unwrap().outputs.is_none());
    }

    #[rstest::rstest]
    fn seed_inputs_reads_upstream_outputs() {
        let graph = linear_graph();
        let execution = WorkflowExecution::new(graph);

        // Simulate: a completed with output.
        execution.set_status("a", NodeStatus::Completed);
        let mut a_out = PortValues::new();
        a_out.insert(
            "out".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        execution.set_node_outputs("a", a_out);

        // Seed b's inputs from a's outputs.
        execution.seed_inputs("b");

        let snap = execution.snapshot();
        let b_state = snap.node_state("b").expect("b exists");
        let b_inputs = b_state.inputs.as_ref().expect("inputs seeded");
        assert_eq!(
            b_inputs.get("in"),
            Some(&PortValue::Single(ScalarValue::Text("hello".to_owned())))
        );
    }
}
