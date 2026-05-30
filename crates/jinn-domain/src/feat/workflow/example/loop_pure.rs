//! Pure-logic loop example — iterative counter with judge validation.
//!
//! A 3-node body graph inside a LoopNode:
//! 1. **source** — emits a starting text value
//! 2. **transform** — appends ".X" to the text each iteration
//! 3. **judge** — outputs "pass" when the text has 3+ ".X" suffixes, "fail" otherwise
//!
//! The LoopNode runs the body graph up to 5 times. Feedback from the transform's
//! output overrides the source node's output on subsequent iterations.

use jinn_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use jinn_workflow::node::code::CodeNode;
use jinn_workflow::node::loop_node::LoopNode;
use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers the loop_pure example workflow.
///
/// # Panics
///
/// Never panics — registration is infallible.
pub fn register(registry: &mut WorkflowRegistry) {
    registry.register("loop_pure", build_loop_pure);
}

/// Builds the "loop_pure" workflow graph.
///
/// Demonstrates:
/// - LoopNode wrapping a body graph with iterative refinement
/// - Feedback injection (transform output → source input)
/// - Exit condition via regex on a judge node's output
/// - Pure logic (no LLM dependency)
///
/// # Panics
///
/// Panics if the static graph definition is invalid (should never happen).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_loop_pure() -> WorkflowGraph {
    let loop_node = LoopNode::new(
        "refine_loop".to_owned(),
        vec![],
        vec![PortDef::text("result")],
        Box::new(build_body_graph),
    )
    .with_exit_condition("judge".to_owned(), "verdict".to_owned(), "pass")
    .with_max_iterations(5)
    .with_feedback(
        "transform".to_owned(),
        "out".to_owned(),
        "source".to_owned(),
        "text".to_owned(),
    )
    .with_output_mapping("result".to_owned(), "transform".to_owned(), "out".to_owned());

    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("loop".to_owned(), Box::new(loop_node));
    builder
        .with_description("Pure-logic loop: iterative transform with judge exit condition")
        .build()
        .expect("loop_pure graph should be valid")
}

/// Builds the body graph for the loop.
///
/// Creates fresh nodes each call since nodes are not Clone.
fn build_body_graph() -> WorkflowGraph {
    let source = CodeNode::new(
        "source".to_owned(),
        vec![],
        vec![PortDef::text("text")],
        |_inputs, _ctx| {
            Box::pin(async move {
                let mut out = PortValues::new();
                out.insert(
                    "text".to_owned(),
                    PortValue::single(ScalarValue::Text("start".to_owned())),
                );
                Ok(out)
            })
        },
    );

    let transform = CodeNode::new(
        "transform".to_owned(),
        vec![PortDef::text("in")],
        vec![PortDef::text("out")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let text = inputs
                    .take_text("in")
                    .map_err(|_e| error_stack::Report::new(jinn_workflow::node::NodeError))?;
                let mut out = PortValues::new();
                out.insert(
                    "out".to_owned(),
                    PortValue::single(ScalarValue::Text(format!("{text}.X"))),
                );
                Ok(out)
            })
        },
    );

    let judge = CodeNode::new(
        "judge".to_owned(),
        vec![PortDef::text("in")],
        vec![PortDef::text("verdict")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let text = inputs
                    .take_text("in")
                    .map_err(|_e| error_stack::Report::new(jinn_workflow::node::NodeError))?;
                let x_count = text.matches(".X").count();
                let verdict = if x_count >= 3 { "pass" } else { "fail" };
                let mut out = PortValues::new();
                out.insert(
                    "verdict".to_owned(),
                    PortValue::single(ScalarValue::Text(verdict.to_owned())),
                );
                Ok(out)
            })
        },
    );

    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("source".to_owned(), Box::new(source));
    builder.add_node("transform".to_owned(), Box::new(transform));
    builder.add_node("judge".to_owned(), Box::new(judge));
    builder
        .connect("source", "text", "transform", "in")
        .expect("source → transform");
    builder
        .connect("transform", "out", "judge", "in")
        .expect("transform → judge");
    builder.build().expect("body graph should build")
}
