//! Pipeline example — chain of text processing nodes into an LLM.
//!
//! A 4-node pipeline:
//! 1. **source** — emits a hard-coded topic string
//! 2. **formatter** — wraps the topic in a prompt template
//! 3. **llm** — sends the prompt to the LLM and returns the response
//! 4. **transform** — uppercases the first line (simulates post-processing)

use jinn_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use jinn_workflow::node::code::CodeNode;
use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

use crate::feat::workflow::node::LlmNode;
use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers the pipeline example workflow.
///
/// # Panics
///
/// Never panics — registration is infallible.
pub fn register(registry: &mut WorkflowRegistry) {
    registry.register("pipeline", build_pipeline);
}

/// Builds the "pipeline" workflow graph.
///
/// Demonstrates:
/// - A `CodeNode` source emitting a string
/// - A `CodeNode` that transforms text (template formatting)
/// - An `LlmNode` that sends the prompt to the LLM
/// - A `CodeNode` that post-processes the response
///
/// # Panics
///
/// Panics if the static graph definition is invalid (should never happen).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_pipeline() -> WorkflowGraph {
    // Node 1: Source — emits a topic string.
    let source = CodeNode::new(
        "source".to_owned(),
        vec![], // no inputs
        vec![PortDef::text("topic")],
        |_inputs, _ctx| {
            Box::pin(async move {
                let mut out = PortValues::new();
                out.insert(
                    "topic".to_owned(),
                    PortValue::single(ScalarValue::Text(
                        "the history of Rust programming language".to_owned(),
                    )),
                );
                Ok(out)
            })
        },
    );

    // Node 2: Formatter — wraps the topic in a prompt template.
    let formatter = CodeNode::new(
        "formatter".to_owned(),
        vec![PortDef::text("topic")],
        vec![PortDef::text("prompt")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let topic = inputs
                    .take_text("topic")
                    .map_err(|_e| error_stack::Report::new(jinn_workflow::node::NodeError))?;
                let prompt = format!("Write a one-paragraph summary about: {topic}");
                let mut out = PortValues::new();
                out.insert("prompt".to_owned(), prompt.into());
                Ok(out)
            })
        },
    );

    // Node 3: LLM — receives the formatted prompt and calls the LLM.
    let llm =
        LlmNode::new("You are a concise technical writer. Provide accurate, brief summaries.");

    // Node 4: Transform — uppercases the first line of the response.
    let transform = CodeNode::new(
        "transform".to_owned(),
        vec![PortDef::text("response")],
        vec![PortDef::text("result")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let response = inputs
                    .take_text("response")
                    .map_err(|_e| error_stack::Report::new(jinn_workflow::node::NodeError))?;
                let result = uppercase_first_line(&response);
                let mut out = PortValues::new();
                out.insert("result".to_owned(), result.into());
                Ok(out)
            })
        },
    );

    // Wire them up:
    //   source.topic → formatter.topic → formatter.prompt → llm.user
    //   llm.response → transform.response → transform.result
    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("source".to_owned(), Box::new(source));
    builder.add_node("formatter".to_owned(), Box::new(formatter));
    builder.add_node("llm".to_owned(), Box::new(llm));
    builder.add_node("transform".to_owned(), Box::new(transform));

    builder
        .connect("source", "topic", "formatter", "topic")
        .expect("source.topic → formatter.topic");
    builder
        .connect("formatter", "prompt", "llm", "user")
        .expect("formatter.prompt → llm.user");
    builder
        .connect("llm", "response", "transform", "response")
        .expect("llm.response → transform.response");

    builder
        .with_description("4-node text processing pipeline with LLM step")
        .build()
        .expect("pipeline graph should be valid")
}

/// Upper-cases the first line of the input text.
fn uppercase_first_line(text: &str) -> String {
    let Some((first, rest)) = text.split_once('\n') else {
        return text.to_uppercase();
    };
    format!("{}\n{rest}", first.to_uppercase())
}
