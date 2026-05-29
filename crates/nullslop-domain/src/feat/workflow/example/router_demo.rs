//! Router demonstration example — conditional branching.
//!
//! A 4-node workflow demonstrating the RouterNode:
//! 1. **source** — emits "yes" or "no"
//! 2. **router** — routes to "yes" or "no" output port based on input
//! 3. **upper** — uppercases the input (on "yes" branch)
//! 4. **reverse** — reverses the input (on "no" branch)
//!
//! Only the matching branch executes. The other branch's nodes are `Skipped`.

use nullslop_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use nullslop_workflow::node::code::CodeNode;
use nullslop_workflow::node::router::RouterNode;
use nullslop_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

use crate::feat::workflow::workflow_registry::WorkflowRegistry;

/// Registers the router_demo example workflow.
///
/// # Panics
///
/// Never panics — registration is infallible.
pub fn register(registry: &mut WorkflowRegistry) {
    registry.register("router_demo", build_router_demo);
}

/// Builds the "router_demo" workflow graph.
///
/// Demonstrates:
/// - RouterNode with two output ports ("yes", "no")
/// - Conditional branching: only matching downstream nodes execute
/// - Deadlock detection: non-matching branch nodes are marked Skipped
///
/// # Panics
///
/// Panics if the static graph definition is invalid (should never happen).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_router_demo() -> WorkflowGraph {
    // Node 1: Source — emits a fixed value to route on.
    let source = CodeNode::new(
        "source".to_owned(),
        vec![],
        vec![PortDef::text("choice")],
        |_inputs, _ctx| {
            Box::pin(async move {
                let mut out = PortValues::new();
                out.insert(
                    "choice".to_owned(),
                    PortValue::single(ScalarValue::Text("yes".to_owned())),
                );
                Ok(out)
            })
        },
    );

    // Node 2: Router — routes based on the source value.
    let router = RouterNode::new(
        "router".to_owned(),
        PortDef::text("input"),
        vec![PortDef::text("yes"), PortDef::text("no")],
    )
    .with_route("yes".to_owned(), r"(?i)^yes$")
    .with_route("no".to_owned(), r"(?i)^no$");

    // Node 3: Upper — uppercases the value on the "yes" branch.
    let upper = CodeNode::new(
        "upper".to_owned(),
        vec![PortDef::text("in")],
        vec![PortDef::text("out")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let text = inputs
                    .take_text("in")
                    .map_err(|_e| error_stack::Report::new(nullslop_workflow::node::NodeError))?;
                let mut out = PortValues::new();
                out.insert(
                    "out".to_owned(),
                    PortValue::single(ScalarValue::Text(text.to_uppercase())),
                );
                Ok(out)
            })
        },
    );

    // Node 4: Reverse — reverses the value on the "no" branch.
    let reverse = CodeNode::new(
        "reverse".to_owned(),
        vec![PortDef::text("in")],
        vec![PortDef::text("out")],
        |mut inputs, _ctx| {
            Box::pin(async move {
                let text = inputs
                    .take_text("in")
                    .map_err(|_e| error_stack::Report::new(nullslop_workflow::node::NodeError))?;
                let mut out = PortValues::new();
                out.insert(
                    "out".to_owned(),
                    PortValue::single(ScalarValue::Text(text.chars().rev().collect::<String>())),
                );
                Ok(out)
            })
        },
    );

    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("source".to_owned(), Box::new(source));
    builder.add_node("router".to_owned(), Box::new(router));
    builder.add_node("upper".to_owned(), Box::new(upper));
    builder.add_node("reverse".to_owned(), Box::new(reverse));

    // Source → Router
    builder
        .connect("source", "choice", "router", "input")
        .expect("source → router");

    // Router → Upper (yes branch)
    builder
        .connect("router", "yes", "upper", "in")
        .expect("router.yes → upper");

    // Router → Reverse (no branch)
    builder
        .connect("router", "no", "reverse", "in")
        .expect("router.no → reverse");

    builder
        .with_description("Router demo: conditional branching with yes/no paths")
        .build()
        .expect("router_demo graph should be valid")
}
