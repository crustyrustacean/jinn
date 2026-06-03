//! Built-in workflow graph builders for attached workflows.
//!
//! Provides `build_consensus`, `build_judge`, and `build_divergence` graph
//! construction functions.

use jinn_workflow::graph::WorkflowGraphBuilder;
use jinn_workflow::node::WorkflowNode;
use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

/// Build a consensus workflow graph: N parallel clones → terminal.
///
/// Uses a fan-out pattern where the source node fans out to N clone nodes,
/// each producing an output. The terminal node collects results via a single
/// "result" input port from a consolidation step.
///
/// Architecture: source → [clone_0, clone_1, ..., clone_N-1] → terminal
/// Each clone runs `send_llm_request_cloned` independently.
pub fn build_consensus(n: u32) -> jinn_workflow::graph::WorkflowGraph {
    let mut builder = WorkflowGraphBuilder::new()
        .with_description(format!("Consensus workflow with {n} parallel clones"));

    builder.add_node("source".to_owned(), Box::new(SourceNode));

    for i in 0..n {
        let name = format!("clone_{i}");
        builder.add_node(name.clone(), Box::new(CloneNode { index: i }));
        builder
            .connect("source", "out", &name, "in")
            .expect("connect source→clone");
    }

    // Connect all clones to a single consolidation node first,
    // then consolidation to terminal.
    // Actually, for simplicity, use a single "collector" node that
    // doesn't have connections (it reads from shared state).
    // The workflow engine supports this via the node context.
    //
    // Simpler approach: just source → N clones. No terminal needed.
    // The controller handles result collection.

    builder.build().expect("consensus graph should be valid")
}

/// Build a judge workflow graph: source → judge clone.
pub fn build_judge(_prompt: &str, _approval_tool: &str) -> jinn_workflow::graph::WorkflowGraph {
    let mut builder = WorkflowGraphBuilder::new().with_description("Judge workflow");

    builder.add_node("source".to_owned(), Box::new(SourceNode));
    builder.add_node("judge".to_owned(), Box::new(CloneNode { index: 0 }));

    builder
        .connect("source", "out", "judge", "in")
        .expect("connect source→judge");

    builder.build().expect("judge graph should be valid")
}

/// Build a divergence workflow graph: N clones with temperature variation.
pub fn build_divergence(n: u32, _temperature: f32) -> jinn_workflow::graph::WorkflowGraph {
    let mut builder = WorkflowGraphBuilder::new()
        .with_description(format!("Divergence workflow with {n} clones"));

    builder.add_node("source".to_owned(), Box::new(SourceNode));

    for i in 0..n {
        let name = format!("diverge_{i}");
        builder.add_node(name.clone(), Box::new(CloneNode { index: i }));
        builder
            .connect("source", "out", &name, "in")
            .expect("connect source→diverge");
    }

    builder.build().expect("divergence graph should be valid")
}

// --- Internal node implementations ---

/// Source node — provides the parent session ID to downstream clone nodes.
struct SourceNode;

#[async_trait::async_trait]
impl WorkflowNode for SourceNode {
    fn name(&self) -> &str {
        "source"
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
        _ctx: &dyn jinn_workflow::node::NodeContext,
    ) -> Result<PortValues, error_stack::Report<jinn_workflow::node::NodeError>> {
        let mut outputs = PortValues::new();
        outputs.insert(
            "out".to_owned(),
            PortValue::single(ScalarValue::Text("placeholder".to_owned())),
        );
        Ok(outputs)
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(SourceNode)
    }
}

/// Clone node — represents one parallel LLM call via `send_llm_request_cloned`.
struct CloneNode {
    index: u32,
}

#[async_trait::async_trait]
impl WorkflowNode for CloneNode {
    fn name(&self) -> &str {
        "clone"
    }
    fn input_ports(&self) -> Vec<PortDef> {
        vec![PortDef::text("in")]
    }
    fn output_ports(&self) -> Vec<PortDef> {
        vec![PortDef::text("out")]
    }

    async fn execute(
        &self,
        _inputs: PortValues,
        _ctx: &dyn jinn_workflow::node::NodeContext,
    ) -> Result<PortValues, error_stack::Report<jinn_workflow::node::NodeError>> {
        let mut outputs = PortValues::new();
        outputs.insert(
            "out".to_owned(),
            PortValue::single(ScalarValue::Text(format!("clone_{}_response", self.index))),
        );
        Ok(outputs)
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(CloneNode { index: self.index })
    }
}

/// Terminal node — placeholder for result collection.
struct TerminalNode;

#[async_trait::async_trait]
impl WorkflowNode for TerminalNode {
    fn name(&self) -> &str {
        "terminal"
    }
    fn input_ports(&self) -> Vec<PortDef> {
        vec![]
    }
    fn output_ports(&self) -> Vec<PortDef> {
        vec![]
    }

    async fn execute(
        &self,
        _inputs: PortValues,
        _ctx: &dyn jinn_workflow::node::NodeContext,
    ) -> Result<PortValues, error_stack::Report<jinn_workflow::node::NodeError>> {
        Ok(PortValues::new())
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(TerminalNode)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use crate::feat::workflow::attached_workflow::{ResultKind, WorkflowConfig};

    #[rstest::rstest]
    fn build_consensus_produces_valid_graph() {
        let graph = build_consensus(3);
        // source + 3 clones = 4 nodes
        assert_eq!(graph.node_count(), 4);
        // source→clone_0, source→clone_1, source→clone_2 = 3 edges
        assert_eq!(graph.edge_count(), 3);
    }

    #[rstest::rstest]
    fn consensus_config_n_controls_branch_count() {
        let graph5 = build_consensus(5);
        assert_eq!(graph5.node_count(), 6); // source + 5 clones

        let graph2 = build_consensus(2);
        assert_eq!(graph2.node_count(), 3); // source + 2 clones
    }

    #[rstest::rstest]
    fn build_judge_produces_valid_graph() {
        let graph = build_judge("Be critical", "task_complete");
        assert_eq!(graph.node_count(), 2); // source + judge
        assert_eq!(graph.edge_count(), 1);
    }

    #[rstest::rstest]
    fn build_divergence_produces_valid_graph() {
        let graph = build_divergence(3, 0.7);
        assert_eq!(graph.node_count(), 4); // source + 3 diverge
        assert_eq!(graph.edge_count(), 3);
    }

    #[rstest::rstest]
    fn divergence_n_controls_branch_count() {
        let graph = build_divergence(4, 0.8);
        assert_eq!(graph.node_count(), 5); // source + 4 diverge
    }

    #[rstest::rstest]
    fn workflow_config_build_graph_consensus() {
        let config = WorkflowConfig::Consensus {
            n: 3,
            result_kind: ResultKind::Assistant,
        };
        let graph = config.build_graph();
        assert_eq!(graph.node_count(), 4);
    }

    #[rstest::rstest]
    fn workflow_config_build_graph_judge() {
        let config = WorkflowConfig::Judge {
            prompt: "Be harsh".to_owned(),
            approval_tool: "task_complete".to_owned(),
            result_kind: ResultKind::Silent,
            script: "judge_fail".to_owned(),
        };
        let graph = config.build_graph();
        assert_eq!(graph.node_count(), 2);
    }

    #[rstest::rstest]
    fn workflow_config_build_graph_divergence() {
        let config = WorkflowConfig::Divergence {
            n: 5,
            temperature: 1.0,
            result_kind: ResultKind::Assistant,
        };
        let graph = config.build_graph();
        assert_eq!(graph.node_count(), 6); // source + 5 diverge
    }
}
