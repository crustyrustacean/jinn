//! LLM-based loop example - iterative generator + judge refinement.
//!
//! A 2-node body graph inside a LoopNode:
//! 1. **generator** - LlmNode that generates text based on a prompt
//! 2. **judge** - LlmNode with YES/NO validation that evaluates the generator's output
//!
//! The LoopNode runs the body graph up to 3 times. Feedback from the judge's
//! response overrides the generator's prompt on subsequent iterations.
//! Exit condition: judge outputs "YES" on its response port.

use jinn_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use jinn_workflow::node::loop_node::LoopNode;
use jinn_workflow::port::PortDef;

use crate::feat::workflow::node::LlmNode;
use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers the loop_llm example workflow.
///
/// # Panics
///
/// Never panics - registration is infallible.
pub fn register(registry: &mut WorkflowRegistry) {
    registry.register("loop_llm", build_loop_llm);
}

/// Builds the "loop_llm" workflow graph.
///
/// Demonstrates:
/// - LoopNode wrapping two LLM nodes (generator + judge)
/// - LlmNode with validation regex for YES/NO responses
/// - Feedback injection: judge response → generator prompt
/// - Exit condition via regex on judge's response
/// - Full LLM integration inside a loop
///
/// # Panics
///
/// Panics if the static graph definition is invalid (should never happen).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_loop_llm() -> WorkflowGraph {
    let loop_node = LoopNode::new(
        "refine_loop".to_owned(),
        vec![],
        vec![PortDef::text("result")],
        Box::new(build_body_graph),
    )
    .with_exit_condition("judge".to_owned(), "response".to_owned(), "(?i)^yes")
    .with_max_iterations(3)
    .with_feedback(
        "judge".to_owned(),
        "response".to_owned(),
        "generator".to_owned(),
        "user".to_owned(),
    )
    .with_output_mapping("result".to_owned(), "generator".to_owned(), "response".to_owned());

    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("loop".to_owned(), Box::new(loop_node));
    builder
        .with_description("LLM loop: generator + judge with iterative refinement")
        .build()
        .expect("loop_llm graph should be valid")
}

/// Builds the body graph for the LLM loop.
///
/// Creates fresh nodes each call since nodes are not Clone.
fn build_body_graph() -> WorkflowGraph {
    // Generator: LLM that produces text from a prompt.
    let generator = LlmNode::new("You are a helpful assistant. Write a clear, concise response.");

    // Judge: LLM that evaluates the generator's output.
    // Uses validation regex to ensure YES/NO response.
    let judge = LlmNode::new(
        "You are a quality judge. Evaluate the previous response. \
         Respond with YES if the response is good, or NO if it needs improvement.",
    )
    .with_validation(
        r"^(?i)(YES|NO)",
        2,
        "Your response must start with YES or NO. Try again.",
    );

    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("generator".to_owned(), Box::new(generator));
    builder.add_node("judge".to_owned(), Box::new(judge));

    // Generator's response → Judge's input.
    builder
        .connect("generator", "response", "judge", "user")
        .expect("generator.response → judge.user");

    builder.build().expect("body graph should build")
}
