//! Workflow execution engine.
//!
//! Takes a validated [`WorkflowGraph`](crate::graph::WorkflowGraph) and executes it
//! using a topological push model. Source nodes run first, their outputs propagate
//! through edges to downstream nodes, and independent branches execute concurrently.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use error_stack::Report;
use petgraph::visit::EdgeRef;
use tokio_util::sync::CancellationToken;
use wherror::Error;

use crate::graph::WorkflowGraph;
use crate::node::{NodeContext, WorkflowNode};
use crate::port::PortValues;

/// Status of a single node during and after execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// Not yet started (waiting for inputs).
    Pending,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Completed,
    /// Failed with an error.
    Failed,
    /// Skipped because an upstream node failed.
    Skipped,
}

impl NodeStatus {
    /// Returns `true` if the node is in a terminal state (no further transitions).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Skipped)
    }
}

/// The result of running a workflow to completion.
#[derive(Debug)]
pub struct WorkflowResult {
    /// Per-node output values (only for Completed nodes).
    pub outputs: HashMap<String, PortValues>,
    /// Per-node status.
    pub statuses: HashMap<String, NodeStatus>,
}

/// Error type for engine operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct EngineError;

/// Internal message sent when a node completes execution.
enum CompletionMsg {
    /// The node completed successfully.
    Success {
        /// Node name.
        name: String,
        /// Output port values.
        outputs: PortValues,
    },
    /// The node failed (returned Err or panicked).
    Failed {
        /// Node name.
        name: String,
    },
}

/// Executes a validated workflow graph.
///
/// Takes ownership of the graph (nodes are consumed). Returns when all
/// reachable nodes have completed or failed.
///
/// # Errors
///
/// Returns an error if the engine encounters an internal failure.
pub async fn execute(
    graph: WorkflowGraph,
    ctx: Arc<dyn NodeContext>,
) -> Result<WorkflowResult, Report<EngineError>> {
    execute_with_cancel(graph, ctx, CancellationToken::new()).await
}

/// Executes a workflow with cancellation support.
///
/// When the token is cancelled, all running node tasks are aborted.
/// Running nodes are marked `Failed` and pending nodes are marked `Skipped`.
///
/// # Errors
///
/// Returns an error if the engine encounters an internal failure.
pub async fn execute_with_cancel(
    graph: WorkflowGraph,
    ctx: Arc<dyn NodeContext>,
    cancel: CancellationToken,
) -> Result<WorkflowResult, Report<EngineError>> {
    let inner = graph.inner();
    let name_to_index = graph.name_to_index();

    // Build maps for node lookup.
    let mut node_map: HashMap<String, Box<dyn WorkflowNode>> = HashMap::new();
    let mut node_names: HashMap<petgraph::graph::NodeIndex, String> = HashMap::new();
    for (name, &idx) in name_to_index {
        node_map.insert(name.clone(), inner[idx].node.clone_box());
        node_names.insert(idx, name.clone());
    }

    // Initialize tracking.
    let mut statuses: HashMap<String, NodeStatus> = HashMap::new();
    let mut pending_inputs: HashMap<String, PortValues> = HashMap::new();
    let mut pending_count: HashMap<String, usize> = HashMap::new();
    let mut outputs: HashMap<String, PortValues> = HashMap::new();
    let mut handles: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

    for (name, node) in &node_map {
        statuses.insert(name.clone(), NodeStatus::Pending);
        pending_inputs.insert(name.clone(), PortValues::new());
        let input_port_count = node.input_ports().len();
        pending_count.insert(name.clone(), input_port_count);
    }

    // Channel for completion messages.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<CompletionMsg>(64);

    // Spawn source nodes immediately (they have zero input ports).
    for source_name in graph.sources() {
        spawn_node(
            source_name.clone(),
            &node_map,
            &mut statuses,
            PortValues::new(),
            &ctx,
            &tx,
            &mut handles,
        );
    }

    // Main execution loop.
    let total_nodes = node_map.len();
    let mut completed_count = 0;

    loop {
        // Check if all nodes are terminal.
        if completed_count >= total_nodes {
            break;
        }

        // Build a future that monitors all running handles for panics.
        // If any handle finishes without sending a message, the node panicked.
        let panic_check = async {
            // Poll handles periodically to detect panics.
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };

        tokio::select! {
            msg = rx.recv() => {
                #[expect(clippy::expect_used, reason = "channel cannot close while tasks are pending")]
                let msg = msg.expect("channel should not close while nodes are pending");
                handle_completion(
                    msg,
                    inner,
                    name_to_index,
                    &node_names,
                    &node_map,
                    &mut statuses,
                    &mut pending_inputs,
                    &mut pending_count,
                    &mut outputs,
                    &mut handles,
                    &mut completed_count,
                    &ctx,
                    &tx,
                );
            }
            // Check for panicked tasks: any Running handle that is_finished()
            // means it panicked (normal completions send a message first and
            // remove the handle from the map).
            Some(panic_name) = check_panics_async(&handles, &statuses) => {
                // Remove the handle and mark as failed.
                if let Some(handle) = handles.remove(&panic_name) {
                    handle.abort();
                }
                statuses.insert(panic_name.clone(), NodeStatus::Failed);
                completed_count += 1;

                // Propagate skip to downstream.
                #[expect(clippy::indexing_slicing, reason = "node name is validated during graph construction")]
                let failed_idx = name_to_index[&panic_name];
                let downstream = find_downstream(failed_idx, inner, &node_names);
                for down_name in &downstream {
                    if statuses.get(down_name) == Some(&NodeStatus::Pending) {
                        statuses.insert(down_name.clone(), NodeStatus::Skipped);
                        completed_count += 1;
                    }
                }
            }
            _ = cancel.cancelled() => {
                // Abort all running tasks.
                for (_, handle) in handles.drain() {
                    handle.abort();
                }
                // Mark running as Failed, pending as Skipped.
                for status in statuses.values_mut() {
                    match status {
                        NodeStatus::Running => *status = NodeStatus::Failed,
                        NodeStatus::Pending => *status = NodeStatus::Skipped,
                        _ => {}
                    }
                }
                break;
            }
            () = panic_check => {}
        }
    }

    Ok(WorkflowResult { outputs, statuses })
}

/// Async check: finds a Running node whose handle has finished (panicked).
/// Returns `None` if no such node exists.
async fn check_panics_async(
    handles: &HashMap<String, tokio::task::JoinHandle<()>>,
    statuses: &HashMap<String, NodeStatus>,
) -> Option<String> {
    // Small sleep to avoid busy-looping.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    for (name, handle) in handles {
        if statuses.get(name) == Some(&NodeStatus::Running) && handle.is_finished() {
            return Some(name.clone());
        }
    }
    // If no panic found, yield indefinitely (won't be selected).
    std::future::pending().await
}

/// Handles a node completion message.
#[expect(
    clippy::too_many_arguments,
    reason = "internal helper with many mutable state references"
)]
fn handle_completion(
    msg: CompletionMsg,
    inner: &petgraph::graph::DiGraph<crate::graph::NodeData, crate::graph::EdgeData>,
    name_to_index: &HashMap<String, petgraph::graph::NodeIndex>,
    node_names: &HashMap<petgraph::graph::NodeIndex, String>,
    node_map: &HashMap<String, Box<dyn WorkflowNode>>,
    statuses: &mut HashMap<String, NodeStatus>,
    pending_inputs: &mut HashMap<String, PortValues>,
    pending_count: &mut HashMap<String, usize>,
    outputs: &mut HashMap<String, PortValues>,
    handles: &mut HashMap<String, tokio::task::JoinHandle<()>>,
    completed_count: &mut usize,
    ctx: &Arc<dyn NodeContext>,
    tx: &tokio::sync::mpsc::Sender<CompletionMsg>,
) {
    match msg {
        CompletionMsg::Success {
            name,
            outputs: node_outputs,
        } => {
            statuses.insert(name.clone(), NodeStatus::Completed);
            outputs.insert(name.clone(), node_outputs.clone());
            handles.remove(&name);
            *completed_count += 1;

            // Propagate outputs to downstream nodes.
            let src_idx = name_to_index[&name];
            for edge in inner.edges_directed(src_idx, petgraph::Direction::Outgoing) {
                let tgt_idx = edge.target();
                let tgt_name = &node_names[&tgt_idx];
                let source_port = &edge.weight().source_port;
                let target_port = &edge.weight().target_port;

                // Get the output value for this port.
                if let Some(value) = node_outputs.get(source_port).cloned() {
                    let inputs = pending_inputs.get_mut(tgt_name).expect("node exists");
                    inputs.insert(target_port.clone(), value);
                }

                // Decrement pending count.
                let count = pending_count.get_mut(tgt_name).expect("node exists");
                *count = count.saturating_sub(1);

                // If all inputs satisfied, spawn the downstream node.
                if *count == 0 && statuses.get(tgt_name) == Some(&NodeStatus::Pending) {
                    let inputs = pending_inputs.remove(tgt_name).expect("inputs exist");
                    spawn_node(
                        tgt_name.clone(),
                        node_map,
                        statuses,
                        inputs,
                        ctx,
                        tx,
                        handles,
                    );
                }
            }
        }
        CompletionMsg::Failed { name } => {
            statuses.insert(name.clone(), NodeStatus::Failed);
            handles.remove(&name);
            *completed_count += 1;

            // Propagate skip to all transitive downstream nodes.
            let failed_idx = name_to_index[&name];
            let downstream = find_downstream(failed_idx, inner, node_names);
            for down_name in &downstream {
                if statuses.get(down_name) == Some(&NodeStatus::Pending) {
                    statuses.insert(down_name.clone(), NodeStatus::Skipped);
                    *completed_count += 1;
                }
            }
        }
    }
}

/// Spawns a node execution as a tokio task.
fn spawn_node(
    name: String,
    node_map: &HashMap<String, Box<dyn WorkflowNode>>,
    statuses: &mut HashMap<String, NodeStatus>,
    inputs: PortValues,
    ctx: &Arc<dyn NodeContext>,
    tx: &tokio::sync::mpsc::Sender<CompletionMsg>,
    handles: &mut HashMap<String, tokio::task::JoinHandle<()>>,
) {
    let node = node_map[&name].clone_box();
    let tx = tx.clone();
    let ctx = Arc::clone(ctx);

    statuses.insert(name.clone(), NodeStatus::Running);

    let node_name = name.clone();
    let handle = tokio::spawn(async move {
        let result = node.execute(inputs, &*ctx).await;
        match result {
            Ok(node_outputs) => {
                let _ = tx
                    .send(CompletionMsg::Success {
                        name: node_name,
                        outputs: node_outputs,
                    })
                    .await;
            }
            Err(_) => {
                let _ = tx.send(CompletionMsg::Failed { name: node_name }).await;
            }
        }
    });

    handles.insert(name, handle);
}

/// Finds all transitive downstream node names from a given node index.
fn find_downstream(
    start: petgraph::graph::NodeIndex,
    graph: &petgraph::graph::DiGraph<crate::graph::NodeData, crate::graph::EdgeData>,
    node_names: &HashMap<petgraph::graph::NodeIndex, String>,
) -> HashSet<String> {
    let mut downstream = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(start);

    while let Some(idx) = queue.pop_front() {
        for edge in graph.edges_directed(idx, petgraph::Direction::Outgoing) {
            let tgt = edge.target();
            if let Some(name) = node_names.get(&tgt) {
                if downstream.insert(name.clone()) {
                    queue.push_back(tgt);
                }
            }
        }
    }

    downstream
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::WorkflowGraphBuilder;
    use crate::node::NodeError;
    use crate::port::{PortDef, PortValue};
    use error_stack::Report;
    use std::time::Duration;

    /// A minimal NodeContext for tests.
    struct TestContext;

    impl NodeContext for TestContext {}

    /// Helper: get a status from the result, panicking on missing key.
    fn status(result: &WorkflowResult, name: &str) -> NodeStatus {
        result.statuses.get(name).copied().unwrap_or_else(|| {
            panic!("node '{name}' not found in statuses");
        })
    }

    /// Helper: get outputs for a node, panicking on missing key.
    fn outputs<'a>(result: &'a WorkflowResult, name: &str) -> &'a PortValues {
        result.outputs.get(name).unwrap_or_else(|| {
            panic!("node '{name}' not found in outputs");
        })
    }

    /// Creates a source node that outputs a fixed string.
    fn source_node(output: &'static str) -> Box<dyn WorkflowNode> {
        struct SourceNode {
            output: String,
        }

        #[async_trait::async_trait]
        impl WorkflowNode for SourceNode {
            fn name(&self) -> &'static str {
                "source"
            }
            fn input_ports(&self) -> Vec<PortDef> {
                vec![]
            }
            fn output_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("out")]
            }
            async fn execute(
                &self,
                _inputs: PortValues,
                _ctx: &dyn NodeContext,
            ) -> Result<PortValues, Report<NodeError>> {
                let mut out = PortValues::new();
                out.insert("out".to_owned(), PortValue::String(self.output.clone()));
                Ok(out)
            }
            fn clone_box(&self) -> Box<dyn WorkflowNode> {
                Box::new(SourceNode {
                    output: self.output.clone(),
                })
            }
        }

        Box::new(SourceNode {
            output: output.to_owned(),
        })
    }

    /// Creates a transform node that uppercases the "in" port.
    fn uppercase_node() -> Box<dyn WorkflowNode> {
        struct UpperNode;

        #[async_trait::async_trait]
        impl WorkflowNode for UpperNode {
            fn name(&self) -> &'static str {
                "uppercase"
            }
            fn input_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("in")]
            }
            fn output_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("out")]
            }
            async fn execute(
                &self,
                mut inputs: PortValues,
                _ctx: &dyn NodeContext,
            ) -> Result<PortValues, Report<NodeError>> {
                let val = inputs
                    .take_string("in")
                    .map_err(|_e| Report::new(NodeError))?;
                let mut out = PortValues::new();
                out.insert("out".to_owned(), PortValue::String(val.to_uppercase()));
                Ok(out)
            }
            fn clone_box(&self) -> Box<dyn WorkflowNode> {
                Box::new(UpperNode)
            }
        }

        Box::new(UpperNode)
    }

    /// Creates a transform node that adds a suffix.
    fn suffix_node(suffix: &'static str) -> Box<dyn WorkflowNode> {
        struct SuffixNode {
            suffix: String,
        }

        #[async_trait::async_trait]
        impl WorkflowNode for SuffixNode {
            fn name(&self) -> &'static str {
                "suffix"
            }
            fn input_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("in")]
            }
            fn output_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("out")]
            }
            async fn execute(
                &self,
                mut inputs: PortValues,
                _ctx: &dyn NodeContext,
            ) -> Result<PortValues, Report<NodeError>> {
                let val = inputs
                    .take_string("in")
                    .map_err(|_e| Report::new(NodeError))?;
                let mut out = PortValues::new();
                out.insert(
                    "out".to_owned(),
                    PortValue::String(format!("{val}{suffix}", val = val, suffix = self.suffix)),
                );
                Ok(out)
            }
            fn clone_box(&self) -> Box<dyn WorkflowNode> {
                Box::new(SuffixNode {
                    suffix: self.suffix.clone(),
                })
            }
        }

        Box::new(SuffixNode {
            suffix: suffix.to_owned(),
        })
    }

    /// Creates a fan-in node with two string inputs.
    fn concat_node() -> Box<dyn WorkflowNode> {
        struct ConcatNode;

        #[async_trait::async_trait]
        impl WorkflowNode for ConcatNode {
            fn name(&self) -> &'static str {
                "concat"
            }
            fn input_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("left"), PortDef::string("right")]
            }
            fn output_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("out")]
            }
            async fn execute(
                &self,
                mut inputs: PortValues,
                _ctx: &dyn NodeContext,
            ) -> Result<PortValues, Report<NodeError>> {
                let left = inputs
                    .take_string("left")
                    .map_err(|_e| Report::new(NodeError))?;
                let right = inputs
                    .take_string("right")
                    .map_err(|_e| Report::new(NodeError))?;
                let mut out = PortValues::new();
                out.insert(
                    "out".to_owned(),
                    PortValue::String(format!("{left}+{right}")),
                );
                Ok(out)
            }
            fn clone_box(&self) -> Box<dyn WorkflowNode> {
                Box::new(ConcatNode)
            }
        }

        Box::new(ConcatNode)
    }

    /// Creates a node that always fails.
    fn fail_node() -> Box<dyn WorkflowNode> {
        struct FailNode;

        #[async_trait::async_trait]
        impl WorkflowNode for FailNode {
            fn name(&self) -> &'static str {
                "fail"
            }
            fn input_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("in")]
            }
            fn output_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("out")]
            }
            async fn execute(
                &self,
                _inputs: PortValues,
                _ctx: &dyn NodeContext,
            ) -> Result<PortValues, Report<NodeError>> {
                Err(Report::new(NodeError))
            }
            fn clone_box(&self) -> Box<dyn WorkflowNode> {
                Box::new(FailNode)
            }
        }

        Box::new(FailNode)
    }

    /// Creates a node that delays before passing through.
    fn delay_node(duration: Duration) -> Box<dyn WorkflowNode> {
        struct DelayNode {
            duration: Duration,
        }

        #[async_trait::async_trait]
        impl WorkflowNode for DelayNode {
            fn name(&self) -> &'static str {
                "delay"
            }
            fn input_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("in")]
            }
            fn output_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("out")]
            }
            async fn execute(
                &self,
                mut inputs: PortValues,
                _ctx: &dyn NodeContext,
            ) -> Result<PortValues, Report<NodeError>> {
                tokio::time::sleep(self.duration).await;
                let val = inputs
                    .take_string("in")
                    .map_err(|_e| Report::new(NodeError))?;
                let mut out = PortValues::new();
                out.insert("out".to_owned(), PortValue::String(val));
                Ok(out)
            }
            fn clone_box(&self) -> Box<dyn WorkflowNode> {
                Box::new(DelayNode {
                    duration: self.duration,
                })
            }
        }

        Box::new(DelayNode { duration })
    }

    /// Creates a node that panics inside execute.
    fn panic_node() -> Box<dyn WorkflowNode> {
        struct PanicNode;

        #[async_trait::async_trait]
        impl WorkflowNode for PanicNode {
            fn name(&self) -> &'static str {
                "panic"
            }
            fn input_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("in")]
            }
            fn output_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("out")]
            }
            async fn execute(
                &self,
                _inputs: PortValues,
                _ctx: &dyn NodeContext,
            ) -> Result<PortValues, Report<NodeError>> {
                panic!("intentional panic for testing");
            }
            fn clone_box(&self) -> Box<dyn WorkflowNode> {
                Box::new(PanicNode)
            }
        }

        Box::new(PanicNode)
    }

    #[tokio::test]
    async fn linear_pipeline_flows_data_correctly() {
        // Given A → B → C where A outputs "hello", B uppercases, C adds "-world".
        let ctx = Arc::new(TestContext);
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("hello"))
            .add_node("b".to_owned(), uppercase_node())
            .add_node("c".to_owned(), suffix_node("-world"));
        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("b", "out", "c", "in").expect("b→c");
        let graph = builder.build().expect("build");

        // When executing.
        let result = execute(graph, ctx).await.expect("execute");

        // Then all nodes completed and C's output is "HELLO-world".
        assert_eq!(status(&result, "a"), NodeStatus::Completed);
        assert_eq!(status(&result, "b"), NodeStatus::Completed);
        assert_eq!(status(&result, "c"), NodeStatus::Completed);
        assert_eq!(
            outputs(&result, "c").get_string("out").unwrap(),
            "HELLO-world"
        );
    }

    #[tokio::test]
    async fn fan_out_runs_both_branches() {
        // Given A → B and A → C.
        let ctx = Arc::new(TestContext);
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("data"))
            .add_node("b".to_owned(), uppercase_node())
            .add_node("c".to_owned(), suffix_node("!"));
        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("a", "out", "c", "in").expect("a→c");
        let graph = builder.build().expect("build");

        // When executing.
        let result = execute(graph, ctx).await.expect("execute");

        // Then both branches completed with correct results.
        assert_eq!(status(&result, "b"), NodeStatus::Completed);
        assert_eq!(status(&result, "c"), NodeStatus::Completed);
        assert_eq!(outputs(&result, "b").get_string("out").unwrap(), "DATA");
        assert_eq!(outputs(&result, "c").get_string("out").unwrap(), "data!");
    }

    #[tokio::test]
    async fn fan_in_waits_for_all_inputs() {
        // Given A → C and B → C where C concatenates.
        let ctx = Arc::new(TestContext);
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("left"))
            .add_node("b".to_owned(), source_node("right"))
            .add_node("c".to_owned(), concat_node());
        builder.connect("a", "out", "c", "left").expect("a→c");
        builder.connect("b", "out", "c", "right").expect("b→c");
        let graph = builder.build().expect("build");

        // When executing.
        let result = execute(graph, ctx).await.expect("execute");

        // Then C received both inputs and concatenated them.
        assert_eq!(status(&result, "c"), NodeStatus::Completed);
        assert_eq!(
            outputs(&result, "c").get_string("out").unwrap(),
            "left+right"
        );
    }

    #[tokio::test]
    async fn diamond_fan_out_then_fan_in() {
        // Given A → B → D and A → C → D.
        let ctx = Arc::new(TestContext);
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("test"))
            .add_node("b".to_owned(), uppercase_node())
            .add_node("c".to_owned(), suffix_node("?"))
            .add_node("d".to_owned(), concat_node());
        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("a", "out", "c", "in").expect("a→c");
        builder.connect("b", "out", "d", "left").expect("b→d");
        builder.connect("c", "out", "d", "right").expect("c→d");
        let graph = builder.build().expect("build");

        // When executing.
        let result = execute(graph, ctx).await.expect("execute");

        // Then D received both paths.
        assert_eq!(status(&result, "d"), NodeStatus::Completed);
        assert_eq!(
            outputs(&result, "d").get_string("out").unwrap(),
            "TEST+test?"
        );
    }

    #[tokio::test]
    async fn multi_port_two_inputs_two_outputs() {
        // Given a node with 2 inputs and 2 outputs, each routed independently.
        struct MultiNode;

        #[async_trait::async_trait]
        impl WorkflowNode for MultiNode {
            fn name(&self) -> &'static str {
                "multi"
            }
            fn input_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("in1"), PortDef::string("in2")]
            }
            fn output_ports(&self) -> Vec<PortDef> {
                vec![PortDef::string("out1"), PortDef::string("out2")]
            }
            async fn execute(
                &self,
                mut inputs: PortValues,
                _ctx: &dyn NodeContext,
            ) -> Result<PortValues, Report<NodeError>> {
                let v1 = inputs
                    .take_string("in1")
                    .map_err(|_e| Report::new(NodeError))?;
                let v2 = inputs
                    .take_string("in2")
                    .map_err(|_e| Report::new(NodeError))?;
                let mut out = PortValues::new();
                out.insert("out1".to_owned(), PortValue::String(v1.to_uppercase()));
                out.insert("out2".to_owned(), PortValue::String(format!("{v2}!")));
                Ok(out)
            }
            fn clone_box(&self) -> Box<dyn WorkflowNode> {
                Box::new(MultiNode)
            }
        }

        let ctx = Arc::new(TestContext);
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("hello"))
            .add_node("b".to_owned(), source_node("world"))
            .add_node("d".to_owned(), Box::new(MultiNode))
            .add_node("c".to_owned(), uppercase_node())
            .add_node("e".to_owned(), suffix_node("!!!"));
        builder.connect("a", "out", "d", "in1").expect("a→d");
        builder.connect("b", "out", "d", "in2").expect("b→d");
        builder.connect("d", "out1", "c", "in").expect("d→c");
        builder.connect("d", "out2", "e", "in").expect("d→e");
        let graph = builder.build().expect("build");

        // When executing.
        let result = execute(graph, ctx).await.expect("execute");

        // Then multi-port node routed correctly.
        assert_eq!(status(&result, "d"), NodeStatus::Completed);
        assert_eq!(status(&result, "c"), NodeStatus::Completed);
        assert_eq!(status(&result, "e"), NodeStatus::Completed);
        assert_eq!(outputs(&result, "c").get_string("out").unwrap(), "HELLO");
        assert_eq!(
            outputs(&result, "e").get_string("out").unwrap(),
            "world!!!!"
        );
    }

    #[tokio::test]
    async fn error_propagation_skips_downstream() {
        // Given A → B (fail) → C, and A → D (independent).
        let ctx = Arc::new(TestContext);
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("data"))
            .add_node("b".to_owned(), fail_node())
            .add_node("c".to_owned(), uppercase_node())
            .add_node("d".to_owned(), suffix_node("!"));
        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("b", "out", "c", "in").expect("b→c");
        builder.connect("a", "out", "d", "in").expect("a→d");
        let graph = builder.build().expect("build");

        // When executing.
        let result = execute(graph, ctx).await.expect("execute");

        // Then B failed, C skipped, A and D completed.
        assert_eq!(status(&result, "a"), NodeStatus::Completed);
        assert_eq!(status(&result, "b"), NodeStatus::Failed);
        assert_eq!(status(&result, "c"), NodeStatus::Skipped);
        assert_eq!(status(&result, "d"), NodeStatus::Completed);
        assert_eq!(outputs(&result, "d").get_string("out").unwrap(), "data!");
    }

    #[tokio::test]
    async fn cancellation_aborts_running_nodes() {
        // Given A (source) → B (delay 5s).
        let ctx = Arc::new(TestContext);
        let cancel = CancellationToken::new();
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("data"))
            .add_node("b".to_owned(), delay_node(Duration::from_secs(5)));
        builder.connect("a", "out", "b", "in").expect("a→b");
        let graph = builder.build().expect("build");

        // Cancel after a short delay.
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });

        // When executing with cancellation.
        let result = execute_with_cancel(graph, ctx, cancel)
            .await
            .expect("execute");

        // Then A completed, B failed (cancelled).
        assert_eq!(status(&result, "a"), NodeStatus::Completed);
        assert_eq!(status(&result, "b"), NodeStatus::Failed);
    }

    #[tokio::test]
    async fn async_concurrency_proves_parallel_execution() {
        // Given A → B (500ms delay) and A → C (500ms delay).
        let ctx = Arc::new(TestContext);
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("data"))
            .add_node("b".to_owned(), delay_node(Duration::from_millis(500)))
            .add_node("c".to_owned(), delay_node(Duration::from_millis(500)));
        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("a", "out", "c", "in").expect("a→c");
        let graph = builder.build().expect("build");

        // When executing and measuring wall clock.
        let start = std::time::Instant::now();
        let result = execute(graph, ctx).await.expect("execute");
        let elapsed = start.elapsed();

        // Then both completed and wall clock is less than sum of delays.
        assert_eq!(status(&result, "b"), NodeStatus::Completed);
        assert_eq!(status(&result, "c"), NodeStatus::Completed);
        assert!(
            elapsed < Duration::from_millis(800),
            "expected parallel execution but took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn node_panic_marks_failed_and_skips_downstream() {
        // Given A → B (panic) → C.
        let ctx = Arc::new(TestContext);
        let mut builder = WorkflowGraphBuilder::new();
        builder
            .add_node("a".to_owned(), source_node("data"))
            .add_node("b".to_owned(), panic_node())
            .add_node("c".to_owned(), uppercase_node());
        builder.connect("a", "out", "b", "in").expect("a→b");
        builder.connect("b", "out", "c", "in").expect("b→c");
        let graph = builder.build().expect("build");

        // When executing.
        let result = execute(graph, ctx).await.expect("execute");

        // Then B failed (panicked), C skipped.
        assert_eq!(status(&result, "a"), NodeStatus::Completed);
        assert_eq!(status(&result, "b"), NodeStatus::Failed);
        assert_eq!(status(&result, "c"), NodeStatus::Skipped);
    }
}
