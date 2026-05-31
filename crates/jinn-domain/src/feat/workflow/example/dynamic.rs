//! Dynamic example - graph built entirely from data, no closures.
//!
//! Demonstrates data-driven graph construction using [`NodeRegistry`] and
//! [`DynamicNode`]. The graph topology and node logic come from factory
//! registrations and JSON config - the builder function contains zero
//! Rust closures (no `CodeNode`).
//!
//! Graph: `source` → `uppercase` → `prefix` → `sink`
//!
//! 1. **source** - emits a hard-coded string from config
//! 2. **uppercase** - uppercases the text
//! 3. **prefix** - prepends a configurable label
//! 4. **sink** - accepts the result

use jinn_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use jinn_workflow::node::{DynamicNode, NodeError, WorkflowNode};
use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};
use jinn_workflow::registry::{NodeFactory, NodeRegistry, RegistryError};

use std::sync::Arc;

use error_stack::Report;

use crate::feat::workflow::workflow_registry::WorkflowRegistry;

// --- Node factories ---

/// Factory that creates source nodes emitting a configured text value.
struct SourceFactory;

impl NodeFactory for SourceFactory {
    fn create(
        &self,
        config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
        let output = config
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Report::new(RegistryError::CreationFailed {
                    type_name: "source".to_owned(),
                    reason: "missing 'output' field".to_owned(),
                })
            })?
            .to_owned();

        Ok(Box::new(DynamicNode::new(
            "source",
            vec![],
            vec![PortDef::text("out")],
            Some(config),
            Arc::new(move |_, _| {
                let output = output.clone();
                Box::pin(async move {
                    let mut out = PortValues::new();
                    out.insert(
                        "out".to_owned(),
                        PortValue::Single(ScalarValue::Text(output)),
                    );
                    Ok(out)
                })
            }),
        )))
    }
}

/// Factory that creates uppercase transform nodes.
struct UppercaseFactory;

impl NodeFactory for UppercaseFactory {
    fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
        Ok(Box::new(DynamicNode::new(
            "uppercase",
            vec![PortDef::text("in")],
            vec![PortDef::text("out")],
            None,
            Arc::new(|mut inputs, _| {
                Box::pin(async move {
                    let val = inputs
                        .take_text("in")
                        .map_err(|_e| Report::new(NodeError))?;
                    let mut out = PortValues::new();
                    out.insert(
                        "out".to_owned(),
                        PortValue::Single(ScalarValue::Text(val.to_uppercase())),
                    );
                    Ok(out)
                })
            }),
        )))
    }
}

/// Factory that creates prefix nodes that prepend a configured label.
struct PrefixFactory;

impl NodeFactory for PrefixFactory {
    fn create(
        &self,
        config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
        let label = config
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("RESULT")
            .to_owned();

        Ok(Box::new(DynamicNode::new(
            "prefix",
            vec![PortDef::text("in")],
            vec![PortDef::text("out")],
            Some(config),
            Arc::new(move |mut inputs, _| {
                let label = label.clone();
                Box::pin(async move {
                    let val = inputs
                        .take_text("in")
                        .map_err(|_e| Report::new(NodeError))?;
                    let mut out = PortValues::new();
                    out.insert(
                        "out".to_owned(),
                        PortValue::Single(ScalarValue::Text(format!("{label}: {val}"))),
                    );
                    Ok(out)
                })
            }),
        )))
    }
}

/// Factory that creates sink nodes (accept input, produce nothing).
struct SinkFactory;

impl NodeFactory for SinkFactory {
    fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
        Ok(Box::new(DynamicNode::new(
            "sink",
            vec![PortDef::text("in")],
            vec![],
            None,
            Arc::new(|_, _| Box::pin(async { Ok(PortValues::new()) })),
        )))
    }
}

// --- Build the data-driven graph ---

/// Registers the dynamic example workflow.
pub fn register(registry: &mut WorkflowRegistry) {
    registry.register("dynamic", build_dynamic);
}

/// Builds the "dynamic" workflow graph entirely from the node registry.
///
/// Every node is created via a [`NodeFactory`] and [`DynamicNode`] -
/// no [`CodeNode`](jinn_workflow::node::code::CodeNode) closures.
/// The graph topology and node configuration come from data.
///
/// # Panics
///
/// Panics if the graph definition is invalid (should never happen).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_dynamic() -> WorkflowGraph {
    let node_registry = make_node_registry();

    let mut builder = WorkflowGraphBuilder::new();
    builder
        .add_node_from_registry(
            "source".to_owned(),
            &node_registry,
            "source",
            serde_json::json!({"output": "hello from dynamic graph"}),
        )
        .expect("add source")
        .add_node_from_registry(
            "uppercase".to_owned(),
            &node_registry,
            "uppercase",
            serde_json::json!({}),
        )
        .expect("add uppercase")
        .add_node_from_registry(
            "prefix".to_owned(),
            &node_registry,
            "prefix",
            serde_json::json!({"label": "DYNAMIC"}),
        )
        .expect("add prefix")
        .add_node_from_registry(
            "sink".to_owned(),
            &node_registry,
            "sink",
            serde_json::json!({}),
        )
        .expect("add sink");

    // source → uppercase → prefix → sink
    builder
        .connect("source", "out", "uppercase", "in")
        .expect("source → uppercase");
    builder
        .connect("uppercase", "out", "prefix", "in")
        .expect("uppercase → prefix");
    builder
        .connect("prefix", "out", "sink", "in")
        .expect("prefix → sink");

    builder
        .with_description("Dynamic graph constructed from runtime parameters")
        .build()
        .expect("dynamic graph should be valid")
}

/// Constructs the node registry with all factories for the dynamic workflow.
fn make_node_registry() -> NodeRegistry {
    let mut registry = NodeRegistry::new();
    registry.register("source", Box::new(SourceFactory));
    registry.register("uppercase", Box::new(UppercaseFactory));
    registry.register("prefix", Box::new(PrefixFactory));
    registry.register("sink", Box::new(SinkFactory));
    registry
}
