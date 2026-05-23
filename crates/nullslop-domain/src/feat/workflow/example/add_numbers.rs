//! Trivial example workflow — add two numbers.
//!
//! A 3-node pipeline using `CodeNode`s:
//! 1. **source** — emits two numbers (3 and 7)
//! 2. **add** — sums the two numbers
//! 3. **sink** — outputs the result
//!
//! This verifies the pipeline works end-to-end without LLM complexity.

use nullslop_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use nullslop_workflow::node::code::CodeNode;
use nullslop_workflow::port::{PortDef, PortValue, PortValues};

use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers the example workflows into the given registry.
pub fn register(registry: &mut WorkflowRegistry) {
    registry.register("add-numbers", build_add_numbers);
}

/// Builds the "add-numbers" workflow graph.
///
/// A trivial 3-node pipeline:
/// 1. **source** — emits two hard-coded numbers via "a" and "b" output ports
/// 2. **add** — reads "a" and "b", outputs "sum" = a + b
/// 3. **sink** — reads "sum" and outputs "result"
///
/// # Errors
///
/// Returns an error if the graph fails validation (should not happen with static definitions).
///
/// # Panics
///
/// Panics if graph connections are invalid (should never happen with static graph definition).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_add_numbers() -> WorkflowGraph {
    let source = CodeNode::new(
        "source".to_owned(),
        vec![], // no inputs
        vec![PortDef::string("a"), PortDef::string("b")],
        |_inputs, _ctx| {
            Box::pin(async move {
                let mut out = PortValues::new();
                out.insert("a".to_owned(), PortValue::String("3".to_owned()));
                out.insert("b".to_owned(), PortValue::String("7".to_owned()));
                Ok(out)
            })
        },
    );

    let add = CodeNode::new(
        "add".to_owned(),
        vec![PortDef::string("a"), PortDef::string("b")],
        vec![PortDef::string("sum")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let a: i32 = inputs
                    .take_string("a")
                    .map_err(|_e| error_stack::Report::new(nullslop_workflow::node::NodeError))?
                    .parse()
                    .map_err(|_e| error_stack::Report::new(nullslop_workflow::node::NodeError))?;
                let b: i32 = inputs
                    .take_string("b")
                    .map_err(|_e| error_stack::Report::new(nullslop_workflow::node::NodeError))?
                    .parse()
                    .map_err(|_e| error_stack::Report::new(nullslop_workflow::node::NodeError))?;
                let mut out = PortValues::new();
                out.insert("sum".to_owned(), PortValue::String((a + b).to_string()));
                Ok(out)
            })
        },
    );

    let sink = CodeNode::new(
        "sink".to_owned(),
        vec![PortDef::string("sum")],
        vec![PortDef::string("result")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let sum = inputs
                    .take_string("sum")
                    .map_err(|_e| error_stack::Report::new(nullslop_workflow::node::NodeError))?;
                let mut out = PortValues::new();
                out.insert("result".to_owned(), PortValue::String(sum));
                Ok(out)
            })
        },
    );

    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("source".to_owned(), Box::new(source));
    builder.add_node("add".to_owned(), Box::new(add));
    builder.add_node("sink".to_owned(), Box::new(sink));
    builder
        .connect("source", "a", "add", "a")
        .expect("source.a -> add.a connection should be valid");
    builder
        .connect("source", "b", "add", "b")
        .expect("source.b -> add.b connection should be valid");
    builder
        .connect("add", "sum", "sink", "sum")
        .expect("add.sum -> sink.sum connection should be valid");
    builder
        .build()
        .expect("add-numbers graph should be valid")
}
