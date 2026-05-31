//! Summarize example - minimal LLM workflow.
//!
//! A 2-node pipeline:
//! 1. **source** - emits a hard-coded text to summarize
//! 2. **llm** - sends the text to the LLM with a system prompt and returns the summary

use jinn_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use jinn_workflow::node::code::CodeNode;
use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

use crate::feat::workflow::node::LlmNode;
use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers the summarize example workflow.
///
/// # Panics
///
/// Never panics - registration is infallible.
pub fn register(registry: &mut WorkflowRegistry) {
    registry.register("summarize", build_summarize);
}

/// Builds the "summarize" workflow graph.
///
/// Demonstrates the simplest LLM workflow: a source feeds text directly
/// to the LLM node's `user` port (no `prompt` port needed).
///
/// # Panics
///
/// Panics if the static graph definition is invalid (should never happen).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_summarize() -> WorkflowGraph {
    // Node 1: Source - emits the text to summarize.
    let source = CodeNode::new(
        "source".to_owned(),
        vec![], // no inputs
        vec![PortDef::text("text")],
        |_inputs, _ctx| {
            Box::pin(async move {
                let mut out = PortValues::new();
                out.insert(
                    "text".to_owned(),
                    PortValue::single(ScalarValue::Text(
                        "Rust is a systems programming language sponsored by Mozilla \
                         which describes it as a \"safe, concurrent, practical language\", \
                         supporting functional and imperative-procedural paradigms. \
                         Rust is syntactically similar to C++, but its designers intend \
                         it to provide better memory safety while still maintaining \
                         performance."
                            .to_owned(),
                    )),
                );
                Ok(out)
            })
        },
    );

    // Node 2: LLM - receives the text and summarizes it.
    let llm =
        LlmNode::new("You are a summarizer. Produce a one-sentence summary of the user's text.");

    // Wire: source.text → llm.user
    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("source".to_owned(), Box::new(source));
    builder.add_node("llm".to_owned(), Box::new(llm));

    builder
        .connect("source", "text", "llm", "user")
        .expect("source.text → llm.user");

    builder
        .with_description("LLM-powered summarization workflow")
        .build()
        .expect("summarize graph should be valid")
}
