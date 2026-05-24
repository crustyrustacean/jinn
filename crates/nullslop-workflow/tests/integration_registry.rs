//! Integration tests for registry-based graph construction and execution.
//!
//! These tests verify end-to-end scenarios using the node registry and
//! `DynamicNode` to build and execute graphs from data, not closures.

use std::sync::Arc;
use std::time::Duration;

use error_stack::Report;
use nullslop_workflow::engine::{self, EngineError, NodeStatus};
use nullslop_workflow::execution::WorkflowExecution;
use nullslop_workflow::graph::{GraphError, WorkflowGraphBuilder};
use nullslop_workflow::node::{
    DynamicNode, NodeContext, NodeError, WorkflowNode,
};
use nullslop_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};
use nullslop_workflow::registry::{NodeFactory, NodeRegistry, RegistryError};

/// A minimal NodeContext for tests.
struct TestContext;

impl NodeContext for TestContext {}

/// Helper: get a status from the result.
fn status(result: &engine::WorkflowResult, name: &str) -> NodeStatus {
    result.statuses.get(name).copied().unwrap_or_else(|| {
        panic!("node '{name}' not found in statuses");
    })
}

/// A factory that creates source nodes outputting a fixed string.
struct SourceFactory;

impl NodeFactory for SourceFactory {
    fn create(
        &self,
        config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
        let output = config["output"].as_str().ok_or_else(|| {
            Report::new(RegistryError::CreationFailed {
                type_name: "source".to_owned(),
                reason: "missing 'output' field".to_owned(),
            })
        })?.to_owned();

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

/// A factory that creates uppercase transform nodes.
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

/// A factory that creates sink nodes (accept input, produce nothing).
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

/// A factory that creates a failing node.
struct FailFactory;

impl NodeFactory for FailFactory {
    fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, Report<RegistryError>> {
        Ok(Box::new(DynamicNode::new(
            "fail",
            vec![PortDef::text("in")],
            vec![PortDef::text("out")],
            None,
            Arc::new(|_, _| {
                Box::pin(async { Err(Report::new(NodeError)) })
            }),
        )))
    }
}

fn test_registry() -> NodeRegistry {
    let mut registry = NodeRegistry::new();
    registry.register("source", Box::new(SourceFactory));
    registry.register("uppercase", Box::new(UppercaseFactory));
    registry.register("sink", Box::new(SinkFactory));
    registry.register("fail", Box::new(FailFactory));
    registry
}

#[tokio::test]
async fn end_to_end_registry_graph_execution() {
    // Given a registry with source, uppercase, and sink factories.
    let registry = test_registry();

    // Build graph: source → uppercase → sink
    let mut builder = WorkflowGraphBuilder::new();
    builder
        .add_node_from_registry(
            "src".to_owned(),
            &registry,
            "source",
            serde_json::json!({"output": "hello"}),
        )
        .expect("add source")
        .add_node_from_registry(
            "upper".to_owned(),
            &registry,
            "uppercase",
            serde_json::json!({}),
        )
        .expect("add uppercase")
        .add_node_from_registry(
            "sink".to_owned(),
            &registry,
            "sink",
            serde_json::json!({}),
        )
        .expect("add sink");

    builder.connect("src", "out", "upper", "in").expect("src→upper");
    builder.connect("upper", "out", "sink", "in").expect("upper→sink");

    let graph = builder.build().expect("build");
    let execution = Arc::new(WorkflowExecution::new(graph));

    // When executing.
    let result = engine::execute(execution, Arc::new(TestContext))
        .await
        .expect("execute");

    // Then all nodes completed.
    assert_eq!(status(&result, "src"), NodeStatus::Completed);
    assert_eq!(status(&result, "upper"), NodeStatus::Completed);
    assert_eq!(status(&result, "sink"), NodeStatus::Completed);

    // And source produced "hello".
    let src_outputs = result.outputs.get("src").expect("src outputs");
    assert_eq!(src_outputs.get_text("out").unwrap(), "hello");

    // And uppercase produced "HELLO".
    let upper_outputs = result.outputs.get("upper").expect("upper outputs");
    assert_eq!(upper_outputs.get_text("out").unwrap(), "HELLO");
}

#[tokio::test]
async fn misconstructed_graph_returns_descriptive_error() {
    // Given a builder with nodes but missing connection.
    let registry = test_registry();
    let mut builder = WorkflowGraphBuilder::new();
    builder
        .add_node_from_registry(
            "src".to_owned(),
            &registry,
            "source",
            serde_json::json!({"output": "hello"}),
        )
        .expect("add source")
        .add_node_from_registry(
            "upper".to_owned(),
            &registry,
            "uppercase",
            serde_json::json!({}),
        )
        .expect("add uppercase");

    // No connection from src → upper.

    // When building.
    let result = builder.build();

    // Then it returns DisconnectedInput (never panics).
    assert!(matches!(
        result,
        Err(e) if matches!(e.current_context(), GraphError::DisconnectedInput { .. })
    ));
}

#[tokio::test]
async fn failing_node_skips_downstream() {
    // Given source → fail → sink.
    let registry = test_registry();
    let mut builder = WorkflowGraphBuilder::new();
    builder
        .add_node_from_registry(
            "src".to_owned(),
            &registry,
            "source",
            serde_json::json!({"output": "hello"}),
        )
        .expect("add source")
        .add_node_from_registry(
            "fail".to_owned(),
            &registry,
            "fail",
            serde_json::json!({}),
        )
        .expect("add fail")
        .add_node_from_registry(
            "sink".to_owned(),
            &registry,
            "sink",
            serde_json::json!({}),
        )
        .expect("add sink");

    builder.connect("src", "out", "fail", "in").expect("src→fail");
    builder.connect("fail", "out", "sink", "in").expect("fail→sink");

    let graph = builder.build().expect("build");
    let execution = Arc::new(WorkflowExecution::new(graph));

    // When executing.
    let result = engine::execute(execution, Arc::new(TestContext))
        .await
        .expect("execute");

    // Then source completed, fail failed, sink skipped.
    assert_eq!(status(&result, "src"), NodeStatus::Completed);
    assert_eq!(status(&result, "fail"), NodeStatus::Failed);
    assert_eq!(status(&result, "sink"), NodeStatus::Skipped);
}

#[tokio::test]
async fn cancel_and_resume_registry_graph() {
    // Given a registry with a delay-based source.
    let mut registry = NodeRegistry::new();
    registry.register("source", Box::new(SourceFactory));
    registry.register("sink", Box::new(SinkFactory));

    let mut builder = WorkflowGraphBuilder::new();
    builder
        .add_node_from_registry(
            "src".to_owned(),
            &registry,
            "source",
            serde_json::json!({"output": "data"}),
        )
        .expect("add source")
        .add_node_from_registry(
            "sink".to_owned(),
            &registry,
            "sink",
            serde_json::json!({}),
        )
        .expect("add sink");

    builder.connect("src", "out", "sink", "in").expect("src→sink");

    let graph = builder.build().expect("build");
    let execution = Arc::new(WorkflowExecution::new(graph));

    // When executing.
    let result = engine::execute(execution, Arc::new(TestContext))
        .await
        .expect("execute");

    // Then both completed.
    assert_eq!(status(&result, "src"), NodeStatus::Completed);
    assert_eq!(status(&result, "sink"), NodeStatus::Completed);
}

#[tokio::test]
async fn registry_round_trip() {
    // Given a registry.
    let registry = test_registry();

    // Register factory, create node, add to builder, connect, build, execute.
    let mut builder = WorkflowGraphBuilder::new();
    builder
        .add_node_from_registry(
            "a".to_owned(),
            &registry,
            "source",
            serde_json::json!({"output": "world"}),
        )
        .expect("add a")
        .add_node_from_registry(
            "b".to_owned(),
            &registry,
            "uppercase",
            serde_json::json!({}),
        )
        .expect("add b");

    builder.connect("a", "out", "b", "in").expect("a→b");

    let graph = builder.build().expect("build");
    let execution = Arc::new(WorkflowExecution::new(graph));
    let result = engine::execute(execution, Arc::new(TestContext))
        .await
        .expect("execute");

    assert_eq!(status(&result, "a"), NodeStatus::Completed);
    assert_eq!(status(&result, "b"), NodeStatus::Completed);
    assert_eq!(
        result.outputs.get("b").unwrap().get_text("out").unwrap(),
        "WORLD"
    );
}

#[tokio::test]
async fn wrong_config_returns_error() {
    // Given a registry but wrong config for source (missing "output").
    let registry = test_registry();
    let mut builder = WorkflowGraphBuilder::new();

    let result = builder.add_node_from_registry(
        "src".to_owned(),
        &registry,
        "source",
        serde_json::json!({}),
    );

    // Then it returns CreationFailed.
    assert!(matches!(
        result,
        Err(e) if matches!(e.current_context(), RegistryError::CreationFailed { .. })
    ));
}

#[tokio::test]
async fn unknown_type_returns_error() {
    // Given a registry without "unknown" type.
    let registry = test_registry();
    let mut builder = WorkflowGraphBuilder::new();

    let result = builder.add_node_from_registry(
        "x".to_owned(),
        &registry,
        "unknown",
        serde_json::json!({}),
    );

    // Then it returns NotFound.
    assert!(matches!(
        result,
        Err(e) if matches!(e.current_context(), RegistryError::NotFound { type_name } if type_name == "unknown")
    ));
}

#[tokio::test]
async fn validate_registry_built_graph() {
    // Given a graph built from the registry.
    let registry = test_registry();
    let mut builder = WorkflowGraphBuilder::new();
    builder
        .add_node_from_registry(
            "src".to_owned(),
            &registry,
            "source",
            serde_json::json!({"output": "data"}),
        )
        .expect("add source")
        .add_node_from_registry(
            "sink".to_owned(),
            &registry,
            "sink",
            serde_json::json!({}),
        )
        .expect("add sink");

    builder.connect("src", "out", "sink", "in").expect("connect");

    let graph = builder.build().expect("build");

    // When validating.
    let diagnostics = graph.validate();

    // Then no warnings (fully connected graph).
    assert!(
        diagnostics.is_empty(),
        "expected no warnings, got: {diagnostics:?}"
    );
}
