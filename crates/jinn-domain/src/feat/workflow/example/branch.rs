//! Branch example — parallel fan-out, fan-in.
//!
//! A 5-node pipeline demonstrating concurrent execution:
//! 1. **source** — emits a single topic string
//! 2. **question_writer** — formats the topic as a question (runs in parallel with `outline_writer`)
//! 3. **outline_writer** — formats the topic as an outline request (runs in parallel with `question_writer`)
//! 4. **llm** — receives both prompts concatenated via its `prompt` and `user` inputs
//! 5. **tag** — prepends "[RESULT]" to the LLM response

use jinn_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use jinn_workflow::node::code::CodeNode;
use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

use crate::feat::workflow::node::LlmNode;
use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers the branch example workflow.
///
/// # Panics
///
/// Never panics — registration is infallible.
pub fn register(registry: &mut WorkflowRegistry) {
    registry.register("branch", build_branch);
}

/// Builds the "branch" workflow graph.
///
/// Demonstrates:
/// - Parallel fan-out (source → two independent nodes)
/// - Fan-in (two nodes → single LLM node)
/// - The LLM node's `prompt` port receives one branch, `user` receives the other
///
/// # Panics
///
/// Panics if the static graph definition is invalid (should never happen).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_branch() -> WorkflowGraph {
    // Node 1: Source — emits a topic.
    let source = CodeNode::new(
        "source".to_owned(),
        vec![],
        vec![PortDef::text("topic")],
        |_inputs, _ctx| {
            Box::pin(async move {
                let mut out = PortValues::new();
                out.insert(
                    "topic".to_owned(),
                    PortValue::single(ScalarValue::Text("WebAssembly".to_owned())),
                );
                Ok(out)
            })
        },
    );

    // Node 2: Question writer — produces a question about the topic.
    let question_writer = CodeNode::new(
        "question_writer".to_owned(),
        vec![PortDef::text("topic")],
        vec![PortDef::text("question")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let topic = inputs
                    .take_text("topic")
                    .map_err(|_e| error_stack::Report::new(jinn_workflow::node::NodeError))?;
                let mut out = PortValues::new();
                out.insert(
                    "question".to_owned(),
                    format!("What is {topic} and why does it matter?").into(),
                );
                Ok(out)
            })
        },
    );

    // Node 3: Outline writer — produces an outline request about the topic.
    let outline_writer = CodeNode::new(
        "outline_writer".to_owned(),
        vec![PortDef::text("topic")],
        vec![PortDef::text("outline")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let topic = inputs
                    .take_text("topic")
                    .map_err(|_e| error_stack::Report::new(jinn_workflow::node::NodeError))?;
                let mut out = PortValues::new();
                out.insert(
                    "outline".to_owned(),
                    format!("Provide a brief outline of key points about {topic}.").into(),
                );
                Ok(out)
            })
        },
    );

    // Node 4: LLM — receives question via `prompt` and outline via `user`.
    // LlmNode concatenates prompt + user when both are connected.
    let llm = LlmNode::new("You are a helpful technical educator. Be concise.");

    // Node 5: Tag — prepends a tag to the response.
    let tag = CodeNode::new(
        "tag".to_owned(),
        vec![PortDef::text("response")],
        vec![PortDef::text("result")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let response = inputs
                    .take_text("response")
                    .map_err(|_e| error_stack::Report::new(jinn_workflow::node::NodeError))?;
                let mut out = PortValues::new();
                out.insert("result".to_owned(), format!("[RESULT]\n{response}").into());
                Ok(out)
            })
        },
    );

    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("source".to_owned(), Box::new(source));
    builder.add_node("question_writer".to_owned(), Box::new(question_writer));
    builder.add_node("outline_writer".to_owned(), Box::new(outline_writer));
    builder.add_node("llm".to_owned(), Box::new(llm));
    builder.add_node("tag".to_owned(), Box::new(tag));

    // Fan-out: source → question_writer, source → outline_writer
    builder
        .connect("source", "topic", "question_writer", "topic")
        .expect("source → question_writer");
    builder
        .connect("source", "topic", "outline_writer", "topic")
        .expect("source → outline_writer");

    // Fan-in: question_writer → llm.prompt, outline_writer → llm.user
    builder
        .connect("question_writer", "question", "llm", "prompt")
        .expect("question_writer → llm.prompt");
    builder
        .connect("outline_writer", "outline", "llm", "user")
        .expect("outline_writer → llm.user");

    // LLM → tag
    builder
        .connect("llm", "response", "tag", "response")
        .expect("llm → tag");

    builder
        .with_description("Branching workflow with conditional paths")
        .build()
        .expect("branch graph should be valid")
}
