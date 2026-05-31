//! Loop node - iterative sub-graph execution.
//!
//! [`LoopNode`] wraps a body graph and runs it repeatedly via the engine.
//! Each iteration:
//! 1. Creates a fresh `WorkflowExecution` from the body factory
//! 2. Pre-seeds source nodes with outer inputs (and feedback from prior iteration)
//! 3. Runs the engine to completion
//! 4. Checks an exit condition against a body node's output
//! 5. If met, maps body outputs to the loop's output ports
//!
//! If the body graph has any failed nodes, the loop aborts immediately.
//! If `max_iterations` is exhausted without meeting the exit condition, the loop fails.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use error_stack::Report;
use regex::Regex;
use serde_json::json;
use wherror::Error;

use crate::engine::{self, NodeStatus};
use crate::execution::WorkflowExecution;
use crate::graph::WorkflowGraph;
use crate::node::{NodeContext, NodeError, WorkflowNode};
use crate::port::{PortDef, PortValues};

/// Error type for loop node failures.
#[derive(Debug, Error)]
pub enum LoopError {
    /// The body graph ran for the maximum number of iterations without
    /// the exit condition being met.
    #[error("loop exhausted {max_iterations} iterations without meeting exit condition")]
    MaxIterationsExhausted {
        /// Maximum iterations configured.
        max_iterations: u32,
    },

    /// A node in the body graph failed during execution.
    #[error("body graph node {node} failed")]
    BodyNodeFailed {
        /// Name of the failed node.
        node: String,
    },

    /// The exit condition referenced a node that doesn't exist in the body graph.
    #[error("exit condition references unknown node: {node}")]
    ExitNodeNotFound {
        /// Referenced node name.
        node: String,
    },

    /// The exit condition referenced a port that doesn't exist or has no value.
    #[error("exit condition references missing output: {node}.{port}")]
    ExitPortMissing {
        /// Node name.
        node: String,
        /// Port name.
        port: String,
    },

    /// The context does not support recursive engine execution.
    #[error("node context does not support clone_arc (required for loop execution)")]
    ContextNotCloneable,

    /// The body graph engine returned an error.
    #[error("body graph engine error")]
    EngineError,
}

/// Describes which body node output to check and what pattern means "done".
pub struct ExitCondition {
    /// Node name in the body graph whose output signals loop termination.
    node_name: String,
    /// Port name on that node.
    port_name: String,
    /// Regex pattern. If the port's text value matches, the loop exits.
    pattern: Regex,
}

/// Routes an output from one body node into the input of another for the next iteration.
pub struct FeedbackRoute {
    /// Body node name to read from.
    from_node: String,
    /// Port name to read from.
    from_port: String,
    /// Body node name to write to.
    to_node: String,
    /// Port name to write to.
    to_port: String,
}

/// Maps a body graph node output to the loop's own output port.
pub struct OutputMapping {
    /// Body node name to read from.
    node_name: String,
    /// Port name on that node.
    port_name: String,
}

/// A node that repeatedly executes a sub-graph until an exit condition is met.
///
/// The body graph is created fresh each iteration via a factory closure.
/// The loop's input ports are mapped to body source node inputs on the first iteration.
/// On subsequent iterations, feedback routes override specific body node inputs
/// with values from the previous iteration.
///
/// # Builder API
///
/// See the tests in this crate for full construction examples.
pub struct LoopNode {
    /// Human-readable name for this loop node.
    name: String,
    /// Input ports the loop node declares (outer graph perspective).
    input_ports: Vec<PortDef>,
    /// Output ports the loop node declares (outer graph perspective).
    output_ports: Vec<PortDef>,
    /// Factory that creates a fresh body graph for each iteration.
    body_factory: Box<dyn Fn() -> WorkflowGraph + Send + Sync>,
    /// Maximum number of iterations before failing.
    max_iterations: u32,
    /// Which body node/port to check and what pattern means "exit".
    exit_condition: ExitCondition,
    /// Feedback routes for injecting prior iteration outputs into the next iteration.
    feedback_routes: Vec<FeedbackRoute>,
    /// Maps body graph outputs to the loop's own output ports.
    output_map: HashMap<String, OutputMapping>,
}

impl LoopNode {
    /// Creates a new loop node with the given name, port definitions, and body factory.
    ///
    /// The factory is called once per iteration to produce a fresh graph.
    /// Defaults: `max_iterations = 10`, no feedback routes, no output mappings.
    /// You must call [`with_exit_condition`](Self::with_exit_condition) before execution.
    ///
    /// # Panics
    ///
    /// Panics if the default empty-match regex fails to compile (should never happen).
    #[must_use]
    #[expect(clippy::expect_used, reason = "default regex is compile-time guaranteed to be valid")]
    pub fn new(
        name: String,
        input_ports: Vec<PortDef>,
        output_ports: Vec<PortDef>,
        body_factory: Box<dyn Fn() -> WorkflowGraph + Send + Sync>,
    ) -> Self {
        Self {
            name,
            input_ports,
            output_ports,
            body_factory,
            max_iterations: 10,
            exit_condition: ExitCondition {
                node_name: String::new(),
                port_name: String::new(),
                pattern: Regex::new("$^").expect("empty-match regex always compiles"),
            },
            feedback_routes: Vec::new(),
            output_map: HashMap::new(),
        }
    }

    /// Sets the exit condition: when `node_name`'s `port_name` text output matches
    /// `pattern` (regex), the loop stops and returns its outputs.
    /// # Panics
    ///
    /// Panics if `pattern` is not a valid regex.
    #[must_use]
    #[expect(clippy::panic, reason = "regex validation is caller responsibility")]
    pub fn with_exit_condition(
        mut self,
        node_name: String,
        port_name: String,
        pattern: &str,
    ) -> Self {
        self.exit_condition = ExitCondition {
            node_name,
            port_name,
            pattern: Regex::new(pattern).unwrap_or_else(|e| {
                panic!("invalid exit condition regex `{pattern}`: {e}");
            }),
        };
        self
    }

    /// Sets the maximum number of iterations (default: 10).
    #[must_use]
    pub fn with_max_iterations(mut self, n: u32) -> Self {
        self.max_iterations = n;
        self
    }

    /// Adds a feedback route: after each iteration, the value at
    /// `from_node:from_port` is injected into `to_node:to_port`
    /// for the next iteration.
    #[must_use]
    pub fn with_feedback(
        mut self,
        from_node: String,
        from_port: String,
        to_node: String,
        to_port: String,
    ) -> Self {
        self.feedback_routes.push(FeedbackRoute {
            from_node,
            from_port,
            to_node,
            to_port,
        });
        self
    }

    /// Maps the loop's output port `loop_port` to body node `body_node`'s `body_port`.
    #[must_use]
    pub fn with_output_mapping(
        mut self,
        loop_port: String,
        body_node: String,
        body_port: String,
    ) -> Self {
        self.output_map.insert(
            loop_port,
            OutputMapping {
                node_name: body_node,
                port_name: body_port,
            },
        );
        self
    }

    /// Pre-seeds source nodes (those with no input ports) with matching outer input values.
    ///
    /// For each source node in the body graph, looks at its output port names and
    /// matches them against the loop's outer inputs. Matching values are pre-set
    /// as the source node's outputs so the engine pre-completes them.
    fn seed_source_nodes(execution: &WorkflowExecution, outer_inputs: &PortValues) {
        let snapshot = execution.snapshot();
        let structure = snapshot.structure();

        for node_name in structure.node_names() {
            let Some(input_ports) = structure.node_input_ports(node_name) else {
                continue;
            };
            if input_ports.is_empty() {
                let Some(output_ports) = structure.node_output_ports(node_name) else {
                    continue;
                };
                let mut source_outputs = PortValues::new();
                for port_def in output_ports {
                    if let Some(value) = outer_inputs.get(&port_def.name) {
                        source_outputs.insert(port_def.name.clone(), value.clone());
                    }
                }
                if !source_outputs.is_empty() {
                    execution.set_node_outputs(node_name, source_outputs);
                }
            }
        }
    }

    /// Injects feedback from the previous iteration into the body graph.
    ///
    /// For each feedback route, copies the value from the previous iteration's
    /// outputs into the target node. Source nodes get their outputs updated;
    /// non-source nodes get their inputs updated.
    fn inject_feedback(
        &self,
        execution: &WorkflowExecution,
        prev_outputs: &HashMap<String, PortValues>,
    ) {
        let snapshot = execution.snapshot();
        let structure = snapshot.structure();

        for route in &self.feedback_routes {
            let Some(node_outputs) = prev_outputs.get(&route.from_node) else {
                continue;
            };
            let Some(value) = node_outputs.get(&route.from_port) else {
                continue;
            };

            let target_input_ports = structure.node_input_ports(&route.to_node);
            let is_source = target_input_ports.is_some_and(<[PortDef]>::is_empty);

            if is_source {
                // For source nodes, merge into their existing pre-set outputs.
                let current = execution.snapshot();
                let mut current_outputs: PortValues = current
                    .node_state(&route.to_node)
                    .and_then(|s| s.outputs.as_ref())
                    .map(|arc| (**arc).clone())
                    .unwrap_or_default();
                current_outputs.insert(route.to_port.clone(), value.clone());
                execution.set_node_outputs(&route.to_node, current_outputs);
            } else {
                // For non-source nodes, set their inputs.
                let current = execution.snapshot();
                let mut current_inputs: PortValues = current
                    .node_state(&route.to_node)
                    .and_then(|s| s.inputs.as_ref())
                    .map(|arc| (**arc).clone())
                    .unwrap_or_default();
                current_inputs.insert(route.to_port.clone(), value.clone());
                execution.set_node_inputs(&route.to_node, current_inputs);
            }
        }
    }
}

#[async_trait]
impl WorkflowNode for LoopNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn input_ports(&self) -> Vec<PortDef> {
        self.input_ports.clone()
    }

    fn output_ports(&self) -> Vec<PortDef> {
        self.output_ports.clone()
    }

    #[expect(
        clippy::unimplemented,
        reason = "LoopNode cannot be cloned by design"
    )]
    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        // LoopNode cannot be meaningfully cloned because it contains
        // a factory closure and Regex. The engine should not need to
        // clone LoopNode since it's not used inside itself.
        unimplemented!(
            "LoopNode::clone_box is not supported - loop nodes cannot be cloned"
        )
    }

    fn config(&self) -> Option<serde_json::Value> {
        let feedback: Vec<serde_json::Value> = self
            .feedback_routes
            .iter()
            .map(|r| {
                json!({
                    "from": format!("{}:{}", r.from_node, r.from_port),
                    "to": format!("{}:{}", r.to_node, r.to_port),
                })
            })
            .collect();

        let output_map: Vec<serde_json::Value> = self
            .output_map
            .iter()
            .map(|(loop_port, mapping)| {
                json!({
                    "loop_port": loop_port,
                    "body_node": mapping.node_name,
                    "body_port": mapping.port_name,
                })
            })
            .collect();

        Some(json!({
            "type": "loop",
            "max_iterations": self.max_iterations,
            "exit_condition": {
                "node": self.exit_condition.node_name,
                "port": self.exit_condition.port_name,
                "pattern": self.exit_condition.pattern.to_string(),
            },
            "feedback_routes": feedback,
            "output_map": output_map,
        }))
    }

    async fn execute(
        &self,
        inputs: PortValues,
        ctx: &dyn NodeContext,
    ) -> Result<PortValues, Report<NodeError>> {
        // Get an Arc<dyn NodeContext> for recursive engine execution.
        let ctx_arc = ctx
            .clone_arc()
            .ok_or_else(|| Report::new(NodeError).attach(LoopError::ContextNotCloneable))?;

        let mut prev_body_outputs: Option<HashMap<String, PortValues>> = None;

        for iteration in 0..self.max_iterations {
            // Build a fresh body graph and execute it.
            let graph = (self.body_factory)();
            let execution = Arc::new(WorkflowExecution::new(graph));

            // Seed source nodes with outer inputs.
            Self::seed_source_nodes(&execution, &inputs);

            // Inject feedback from previous iteration (if iteration > 0).
            if iteration > 0 && let Some(ref prev) = prev_body_outputs {
                self.inject_feedback(&execution, prev);
            }

            // Execute the body graph.
            let result = engine::execute(Arc::clone(&execution), Arc::clone(&ctx_arc))
                .await
                .map_err(|_report| Report::new(NodeError).attach(LoopError::EngineError))?;

            // Check for failed nodes in the body graph.
            for (node_name, status) in &result.statuses {
                if *status == NodeStatus::Failed {
                    return Err(Report::new(NodeError).attach(LoopError::BodyNodeFailed {
                        node: node_name.clone(),
                    }));
                }
            }

            // Check exit condition.
            let exit_value = result
                .outputs
                .get(&self.exit_condition.node_name)
                .and_then(|outputs| outputs.get_text(&self.exit_condition.port_name).ok());

            let exit_met = exit_value.is_some_and(|text| {
                self.exit_condition.pattern.is_match(text)
            });

            if exit_met {
                // Extract outputs via output map.
                let mut loop_outputs = PortValues::new();
                for (loop_port, mapping) in &self.output_map {
                    if let Some(node_outputs) = result.outputs.get(&mapping.node_name)
                        && let Some(value) = node_outputs.get(&mapping.port_name)
                    {
                        loop_outputs.insert(loop_port.clone(), value.clone());
                    }
                }
                return Ok(loop_outputs);
            }

            // Exit condition not met - store outputs for feedback and continue.
            prev_body_outputs = Some(result.outputs);
        }

        // Max iterations exhausted.
        Err(Report::new(NodeError).attach(LoopError::MaxIterationsExhausted {
            max_iterations: self.max_iterations,
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::unnecessary_literal_bound,
        reason = "test code"
    )]

    use super::*;
    use std::sync::Mutex;
    use crate::graph::WorkflowGraphBuilder;
    use crate::node::code::CodeNode;
    use crate::node::router::RouterNode;
    use crate::port::{PortDef, PortValue, ScalarValue};

    /// A simple node that echoes its input to its output.
    struct EchoNode;

    #[async_trait]
    impl WorkflowNode for EchoNode {
        fn name(&self) -> &str {
            "echo"
        }
        fn input_ports(&self) -> Vec<PortDef> {
            vec![PortDef::text("in")]
        }
        fn output_ports(&self) -> Vec<PortDef> {
            vec![PortDef::text("out")]
        }
        async fn execute(
            &self,
            mut inputs: PortValues,
            _ctx: &dyn NodeContext,
        ) -> Result<PortValues, Report<NodeError>> {
            let value = inputs.take_text("in").map_err(|_e| Report::new(NodeError))?;
            let mut out = PortValues::new();
            out.insert("out".to_owned(), PortValue::Single(ScalarValue::Text(value)));
            Ok(out)
        }
        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(EchoNode)
        }
    }

    /// A node that produces a fixed output (source node).
    struct FixedOutputNode {
        value: String,
        port_name: String,
    }

    #[async_trait]
    impl WorkflowNode for FixedOutputNode {
        fn name(&self) -> &str {
            "fixed-output"
        }
        fn input_ports(&self) -> Vec<PortDef> {
            vec![]
        }
        fn output_ports(&self) -> Vec<PortDef> {
            vec![PortDef::text(&self.port_name)]
        }
        async fn execute(
            &self,
            _inputs: PortValues,
            _ctx: &dyn NodeContext,
        ) -> Result<PortValues, Report<NodeError>> {
            let mut out = PortValues::new();
            out.insert(
                self.port_name.clone(),
                PortValue::Single(ScalarValue::Text(self.value.clone())),
            );
            Ok(out)
        }
        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(FixedOutputNode {
                value: self.value.clone(),
                port_name: self.port_name.clone(),
            })
        }
    }

    /// A node that appends text to its input.
    #[expect(dead_code, reason = "utility for future tests")]
    struct AppendNode {
        suffix: String,
    }

    #[async_trait]
    impl WorkflowNode for AppendNode {
        fn name(&self) -> &str {
            "append"
        }
        fn input_ports(&self) -> Vec<PortDef> {
            vec![PortDef::text("in")]
        }
        fn output_ports(&self) -> Vec<PortDef> {
            vec![PortDef::text("out")]
        }
        async fn execute(
            &self,
            mut inputs: PortValues,
            _ctx: &dyn NodeContext,
        ) -> Result<PortValues, Report<NodeError>> {
            let value = inputs.take_text("in").map_err(|_e| Report::new(NodeError))?;
            let mut out = PortValues::new();
            out.insert(
                "out".to_owned(),
                PortValue::Single(ScalarValue::Text(format!("{value}{}", self.suffix))),
            );
            Ok(out)
        }
        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(AppendNode {
                suffix: self.suffix.clone(),
            })
        }
    }

    /// A node that always fails.
    struct FailNode;

    #[async_trait]
    impl WorkflowNode for FailNode {
        fn name(&self) -> &str {
            "fail"
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
        ) -> Result<PortValues, Report<NodeError>> {
            Err(Report::new(NodeError))
        }
        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(FailNode)
        }
    }

    /// Test context that supports clone_arc.
    struct TestLoopContext;

    impl NodeContext for TestLoopContext {
        fn clone_arc(&self) -> Option<Arc<dyn NodeContext>> {
            Some(Arc::new(TestLoopContext))
        }
    }

    /// Builds a simple body graph: source → echo → judge.
    /// Source outputs "hello" on port "text".
    /// Echo copies "text" to "out".
    /// Judge is a source node that outputs "pass" on port "verdict"
    /// (matches exit condition on first iter).
    fn build_pass_first_iter_graph() -> WorkflowGraph {
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node(
            "source".to_owned(),
            Box::new(FixedOutputNode {
                value: "hello".to_owned(),
                port_name: "text".to_owned(),
            }),
        );
        builder.add_node("echo".to_owned(), Box::new(EchoNode));
        builder.add_node(
            "judge".to_owned(),
            Box::new(FixedOutputNode {
                value: "pass".to_owned(),
                port_name: "verdict".to_owned(),
            }),
        );
        builder
            .connect("source", "text", "echo", "in")
            .expect("source → echo edge should be valid");
        // judge is a source node (no inputs) - it outputs "pass" directly.
        builder.build().expect("graph should build")
    }

    #[tokio::test]
    async fn loop_exits_on_first_iteration_when_condition_met() {
        // Given a loop node with a body graph that immediately satisfies the exit condition.
        let loop_node = LoopNode::new(
            "test-loop".to_owned(),
            vec![],
            vec![PortDef::text("result")],
            Box::new(build_pass_first_iter_graph),
        )
        .with_exit_condition("judge".to_owned(), "verdict".to_owned(), "pass")
        .with_output_mapping("result".to_owned(), "echo".to_owned(), "out".to_owned())
        .with_max_iterations(5);

        // When executing the loop.
        let ctx = TestLoopContext;
        let result = loop_node
            .execute(PortValues::new(), &ctx)
            .await
            .expect("loop should succeed");

        // Then it returns the mapped output.
        assert_eq!(result.get_text("result").unwrap(), "hello");
    }

    #[tokio::test]
    async fn loop_exhausts_max_iterations() {
        // Given a loop node whose exit condition is never met.
        let loop_node = LoopNode::new(
            "never-done".to_owned(),
            vec![],
            vec![PortDef::text("result")],
            Box::new(build_pass_first_iter_graph),
        )
        .with_exit_condition(
            "judge".to_owned(),
            "verdict".to_owned(),
            "impossible-pattern",
        )
        .with_max_iterations(2);

        // When executing the loop.
        let ctx = TestLoopContext;
        let result = loop_node.execute(PortValues::new(), &ctx).await;

        // Then it fails with max iterations exhausted.
        assert!(result.is_err(), "should fail when max iterations exhausted");
    }

    #[tokio::test]
    async fn loop_propagates_body_failure() {
        // Given a loop node with a body graph that always fails.
        let body_factory = || {
            let mut builder = WorkflowGraphBuilder::new();
            builder.add_node("fail".to_owned(), Box::new(FailNode));
            builder.build().expect("graph should build")
        };

        let loop_node = LoopNode::new(
            "failing-loop".to_owned(),
            vec![],
            vec![PortDef::text("result")],
            Box::new(body_factory),
        )
        .with_exit_condition("fail".to_owned(), "out".to_owned(), "pass")
        .with_max_iterations(3);

        // When executing the loop.
        let ctx = TestLoopContext;
        let result = loop_node.execute(PortValues::new(), &ctx).await;

        // Then it fails.
        assert!(result.is_err(), "should fail when body node fails");
    }

    #[tokio::test]
    async fn loop_config_returns_json() {
        // Given a configured loop node.
        let loop_node = LoopNode::new(
            "refine".to_owned(),
            vec![PortDef::text("prompt")],
            vec![PortDef::text("result")],
            Box::new(build_pass_first_iter_graph),
        )
        .with_exit_condition("judge".to_owned(), "verdict".to_owned(), "pass|approved")
        .with_max_iterations(5)
        .with_feedback(
            "generator".to_owned(),
            "draft".to_owned(),
            "generator".to_owned(),
            "previous".to_owned(),
        )
        .with_output_mapping("result".to_owned(), "generator".to_owned(), "output".to_owned());

        // When getting config.
        let config = loop_node.config().expect("config should return Some");

        // Then it has the expected structure.
        assert_eq!(config["type"], "loop");
        assert_eq!(config["max_iterations"], 5);
        assert_eq!(config["exit_condition"]["node"], "judge");
        assert_eq!(config["exit_condition"]["port"], "verdict");
        assert_eq!(config["exit_condition"]["pattern"], "pass|approved");
        assert!(config["feedback_routes"].is_array());
        assert!(config["output_map"].is_array());
    }

    /// Builds a body graph where a stateful counter tracks iterations.
    /// The judge outputs "pass" only after the 3rd iteration.
    fn build_counter_graph(iteration_count: Arc<Mutex<u32>>) -> WorkflowGraph {
        let count_clone = Arc::clone(&iteration_count);
        let counter = CodeNode::new(
            "counter".to_owned(),
            vec![],
            vec![PortDef::text("count")],
            move |_inputs, _ctx| {
                let count = Arc::clone(&count_clone);
                Box::pin(async move {
                    let mut c = count.lock().unwrap();
                    *c += 1;
                    let mut out = PortValues::new();
                    out.insert(
                        "count".to_owned(),
                        PortValue::Single(ScalarValue::Text(format!("iteration-{}", *c))),
                    );
                    Ok(out)
                })
            },
        );

        let judge = CodeNode::new(
            "judge".to_owned(),
            vec![PortDef::text("_count")],
            vec![PortDef::text("verdict")],
            move |_inputs, _ctx| {
                let c = Arc::clone(&iteration_count);
                Box::pin(async move {
                    let current = *c.lock().unwrap();
                    let verdict = if current >= 3 { "pass" } else { "fail" };
                    let mut out = PortValues::new();
                    out.insert(
                        "verdict".to_owned(),
                        PortValue::Single(ScalarValue::Text(verdict.to_owned())),
                    );
                    Ok(out)
                })
            },
        );

        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node("counter".to_owned(), Box::new(counter));
        builder.add_node("judge".to_owned(), Box::new(judge));
        // Add an edge so counter runs before judge (ensures ordering).
        builder
            .connect("counter", "count", "judge", "_count")
            .expect("counter → judge");
        builder.build().expect("graph should build")
    }

    #[tokio::test]
    async fn loop_runs_multiple_iterations_then_exits() {
        // Given a loop node with a body graph that needs 3 iterations to pass.
        let iteration_count = Arc::new(Mutex::new(0u32));
        let count_for_factory = Arc::clone(&iteration_count);
        let loop_node = LoopNode::new(
            "multi-iter".to_owned(),
            vec![],
            vec![PortDef::text("result")],
            Box::new(move || build_counter_graph(Arc::clone(&count_for_factory))),
        )
        .with_exit_condition("judge".to_owned(), "verdict".to_owned(), "pass")
        .with_output_mapping("result".to_owned(), "counter".to_owned(), "count".to_owned())
        .with_max_iterations(5);

        // When executing the loop.
        let ctx = TestLoopContext;
        let result = loop_node
            .execute(PortValues::new(), &ctx)
            .await
            .expect("loop should succeed after 3 iterations");

        // Then it ran exactly 3 iterations.
        assert_eq!(*iteration_count.lock().unwrap(), 3);
        // And the output was extracted.
        assert_eq!(result.get_text("result").unwrap(), "iteration-3");
    }

    #[tokio::test]
    async fn loop_output_mapping_extracts_correct_values() {
        // Given a loop node with output mapping.
        let iteration_count = Arc::new(Mutex::new(3u32)); // Start at 3 so judge passes immediately
        let count_for_factory = Arc::clone(&iteration_count);
        let loop_node = LoopNode::new(
            "output-test".to_owned(),
            vec![],
            vec![PortDef::text("final_count"), PortDef::text("verdict")],
            Box::new(move || build_counter_graph(Arc::clone(&count_for_factory))),
        )
        .with_exit_condition("judge".to_owned(), "verdict".to_owned(), "pass")
        .with_output_mapping("final_count".to_owned(), "counter".to_owned(), "count".to_owned())
        .with_output_mapping("verdict".to_owned(), "judge".to_owned(), "verdict".to_owned())
        .with_max_iterations(5);

        // When executing.
        let ctx = TestLoopContext;
        let result = loop_node
            .execute(PortValues::new(), &ctx)
            .await
            .expect("loop should succeed");

        // Then both output mappings are populated.
        assert_eq!(result.get_text("final_count").unwrap(), "iteration-4"); // 3+1=4
        assert_eq!(result.get_text("verdict").unwrap(), "pass");
    }

    #[tokio::test]
    async fn loop_feedback_injects_into_next_iteration() {
        // Track what the receiver node gets each iteration.
        let received_inputs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let received_clone = Arc::clone(&received_inputs);
        let make_graph = move || {
            let received = Arc::clone(&received_clone);

            // Source node that produces "seed" on iteration 0.
            let source = CodeNode::new(
                "source".to_owned(),
                vec![],
                vec![PortDef::text("text")],
                move |_inputs, _ctx| {
                    Box::pin(async move {
                        let mut out = PortValues::new();
                        out.insert(
                            "text".to_owned(),
                            PortValue::Single(ScalarValue::Text("seed".to_owned())),
                        );
                        Ok(out)
                    })
                },
            );

            // Receiver node that records its input.
            let received_inner = Arc::clone(&received);
            let receiver_node = CodeNode::new(
                "receiver".to_owned(),
                vec![PortDef::text("in")],
                vec![PortDef::text("out")],
                move |mut inputs, _ctx| {
                    let received = Arc::clone(&received_inner);
                    Box::pin(async move {
                        let value = inputs
                            .take_text("in")
                            .map_err(|_e| error_stack::Report::new(NodeError))?;
                        received.lock().unwrap().push(value.clone());
                        let mut out = PortValues::new();
                        out.insert(
                            "out".to_owned(),
                            PortValue::Single(ScalarValue::Text(format!("processed-{value}"))),
                        );
                        Ok(out)
                    })
                },
            );

            // Judge that passes after 2 iterations.
            let judge = CodeNode::new(
                "judge".to_owned(),
                vec![PortDef::text("in")],
                vec![PortDef::text("verdict")],
                move |mut inputs, _ctx| {
                    Box::pin(async move {
                        let value = inputs
                            .take_text("in")
                            .map_err(|_e| error_stack::Report::new(NodeError))?;
                        // Pass only when we see double-processed feedback (iteration 3+)
                        let verdict = if value.starts_with("processed-processed-") {
                            "pass"
                        } else {
                            "fail"
                        };
                        let mut out = PortValues::new();
                        out.insert(
                            "verdict".to_owned(),
                            PortValue::Single(ScalarValue::Text(verdict.to_owned())),
                        );
                        Ok(out)
                    })
                },
            );

            let mut builder = WorkflowGraphBuilder::new();
            builder.add_node("source".to_owned(), Box::new(source));
            builder.add_node("receiver".to_owned(), Box::new(receiver_node));
            builder.add_node("judge".to_owned(), Box::new(judge));
            builder
                .connect("source", "text", "receiver", "in")
                .expect("source → receiver");
            builder
                .connect("receiver", "out", "judge", "in")
                .expect("receiver → judge");
            builder.build().expect("graph should build")
        };

        let loop_node = LoopNode::new(
            "feedback-test".to_owned(),
            vec![],
            vec![PortDef::text("result")],
            Box::new(make_graph),
        )
        .with_exit_condition("judge".to_owned(), "verdict".to_owned(), "pass")
        .with_max_iterations(5)
        // Feedback: receiver's output -> source's text output (overrides source for next iteration)
        .with_feedback(
            "receiver".to_owned(),
            "out".to_owned(),
            "source".to_owned(),
            "text".to_owned(),
        );

        let ctx = TestLoopContext;
        let result = loop_node.execute(PortValues::new(), &ctx).await;

        // The loop should succeed (judge passes on iteration 2+).
        assert!(result.is_ok(), "loop should succeed: {:?}", result.err());

        // Verify the receiver got input on each iteration.
        let inputs = received_inputs.lock().unwrap();
        // Iteration 1: source seeds "seed" -> receiver gets "seed"
        // Iteration 2: feedback from iter 1's output -> receiver gets "processed-seed"
        assert!(inputs.len() >= 2, "receiver should have at least 2 inputs: {:?}", *inputs);
        assert_eq!(inputs[0], "seed", "first iteration input should be from source");
        assert_eq!(
            inputs[1], "processed-seed",
            "second iteration input should be feedback from first iteration"
        );
    }

    /// Tests that a RouterNode inside a LoopNode body graph works correctly.
    ///
    /// Body graph: source → counter (tracks iteration) → router → (path_a if < 3, path_b if >= 3) → judge
    ///
    /// The router routes to "path_a" for iterations 1-2 and "path_b" for iteration 3+.
    /// Deadlock detection should skip the non-matching branch inside each loop iteration.
    /// The loop exits when the counter reaches 3.
    #[expect(clippy::too_many_lines, reason = "test function with inline graph construction")]
    #[tokio::test]
    async fn router_inside_loop_body_graph() {
        let iteration_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0u32));
        let path_a_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0u32));
        let path_b_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0u32));

        let iter_clone = Arc::clone(&iteration_count);
        let path_a_clone = Arc::clone(&path_a_count);
        let path_b_clone = Arc::clone(&path_b_count);

        let make_graph = move || {
            let iter_c_1 = Arc::clone(&iter_clone);
            let iter_c_2 = Arc::clone(&iter_clone);
            let path_a_c = Arc::clone(&path_a_clone);
            let path_b_c = Arc::clone(&path_b_clone);

            // Counter increments the shared counter and outputs "low" or "high".
            // This is the source node for the body graph.
            let counter = CodeNode::new(
                "counter".to_owned(),
                vec![],
                vec![PortDef::text("level")],
                move |_inputs, _ctx| {
                    let c = Arc::clone(&iter_c_1);
                    Box::pin(async move {
                        let mut val = c.lock().unwrap();
                        *val += 1;
                        let level = if *val < 3 { "low" } else { "high" };
                        let mut out = PortValues::new();
                        out.insert(
                            "level".to_owned(),
                            PortValue::Single(ScalarValue::Text(level.to_owned())),
                        );
                        Ok(out)
                    })
                },
            );

            // Router: routes "low" to path_a, "high" to path_b.
            let router = RouterNode::new(
                "router".to_owned(),
                PortDef::text("input"),
                vec![PortDef::text("path_a"), PortDef::text("path_b")],
            )
            .with_route("path_a".to_owned(), "(?i)^low$")
            .with_route("path_b".to_owned(), "(?i)^high$");

            // Path A handler: records that it ran.
            let path_a = CodeNode::new(
                "path_a".to_owned(),
                vec![PortDef::text("in")],
                vec![PortDef::text("out")],
                move |_inputs, _ctx| {
                    let c = Arc::clone(&path_a_c);
                    Box::pin(async move {
                        *c.lock().unwrap() += 1;
                        let mut out = PortValues::new();
                        out.insert(
                            "out".to_owned(),
                            PortValue::Single(ScalarValue::Text("a-ran".to_owned())),
                        );
                        Ok(out)
                    })
                },
            );

            // Path B handler: records that it ran.
            let path_b = CodeNode::new(
                "path_b".to_owned(),
                vec![PortDef::text("in")],
                vec![PortDef::text("out")],
                move |_inputs, _ctx| {
                    let c = Arc::clone(&path_b_c);
                    Box::pin(async move {
                        *c.lock().unwrap() += 1;
                        let mut out = PortValues::new();
                        out.insert(
                            "out".to_owned(),
                            PortValue::Single(ScalarValue::Text("b-ran".to_owned())),
                        );
                        Ok(out)
                    })
                },
            );

            // Judge: reads counter output and decides pass/fail.
            let judge = CodeNode::new(
                "judge".to_owned(),
                vec![PortDef::text("_level")],
                vec![PortDef::text("verdict")],
                move |_inputs, _ctx| {
                    let c = Arc::clone(&iter_c_2);
                    Box::pin(async move {
                        let current = *c.lock().unwrap();
                        let verdict = if current >= 3 { "pass" } else { "fail" };
                        let mut out = PortValues::new();
                        out.insert(
                            "verdict".to_owned(),
                            PortValue::Single(ScalarValue::Text(verdict.to_owned())),
                        );
                        Ok(out)
                    })
                },
            );

            let mut builder = WorkflowGraphBuilder::new();
            builder.add_node("counter".to_owned(), Box::new(counter));
            builder.add_node("router".to_owned(), Box::new(router));
            builder.add_node("path_a".to_owned(), Box::new(path_a));
            builder.add_node("path_b".to_owned(), Box::new(path_b));
            builder.add_node("judge".to_owned(), Box::new(judge));

            // Counter (source) → Router → path_a (low) / path_b (high)
            // Counter → Judge (ensures ordering)
            builder
                .connect("counter", "level", "router", "input")
                .expect("counter → router");
            builder
                .connect("router", "path_a", "path_a", "in")
                .expect("router.path_a → path_a");
            builder
                .connect("router", "path_b", "path_b", "in")
                .expect("router.path_b → path_b");
            builder
                .connect("counter", "level", "judge", "_level")
                .expect("counter → judge");

            builder.build().expect("graph should build")
        };

        let loop_node = LoopNode::new(
            "router-in-loop".to_owned(),
            vec![],
            vec![PortDef::text("result")],
            Box::new(make_graph),
        )
        .with_exit_condition("judge".to_owned(), "verdict".to_owned(), "pass")
        .with_output_mapping("result".to_owned(), "counter".to_owned(), "level".to_owned())
        .with_max_iterations(5);

        let ctx = TestLoopContext;
        let result = loop_node
            .execute(PortValues::new(), &ctx)
            .await
            .expect("loop with router should succeed");

        // Verify the loop ran 3 iterations.
        assert_eq!(*iteration_count.lock().unwrap(), 3);

        // Verify path_a ran on iterations 1-2 (when counter < 3 → "low").
        assert_eq!(*path_a_count.lock().unwrap(), 2, "path_a should run twice (iters 1-2)");

        // Verify path_b ran on iteration 3 (when counter >= 3 → "high").
        assert_eq!(*path_b_count.lock().unwrap(), 1, "path_b should run once (iter 3)");

        // Verify output mapping.
        assert_eq!(result.get_text("result").unwrap(), "high");
    }
}
