//! Dynamic graph: builds a pipeline entirely from the node registry.
//!
//! Demonstrates constructing a workflow graph from runtime data using
//! `NodeRegistry`, `DynamicNode`, and `add_node_from_registry`. No
//! closures or compile-time node types needed for the graph structure.
//!
//! ```sh
//! cargo run -p nullslop-workflow-tui --example dynamic-graph
//! ```

#[path = "utils/mod.rs"]
mod common;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nullslop_workflow::engine::NodeStatus;
use nullslop_workflow::execution::WorkflowExecution;
use nullslop_workflow::graph::WorkflowGraphBuilder;
use nullslop_workflow::node::{DynamicNode, NodeContext, NodeError, WorkflowNode};
use nullslop_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};
use nullslop_workflow::registry::{NodeFactory, NodeRegistry};
use nullslop_workflow_tui::viewport::ViewportState;
use nullslop_workflow_tui::widget::WorkflowWidget;
use ratatui::widgets::Widget;

/// A factory that creates source nodes outputting a fixed string.
struct SourceFactory;

impl NodeFactory for SourceFactory {
    fn create(
        &self,
        config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, error_stack::Report<nullslop_workflow::registry::RegistryError>>
    {
        let output = config["output"]
            .as_str()
            .ok_or_else(|| {
                error_stack::Report::new(
                    nullslop_workflow::registry::RegistryError::CreationFailed {
                        type_name: "source".to_owned(),
                        reason: "missing 'output' field".to_owned(),
                    },
                )
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

/// A factory that creates passthrough nodes.
struct PassthroughFactory;

impl NodeFactory for PassthroughFactory {
    fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, error_stack::Report<nullslop_workflow::registry::RegistryError>>
    {
        Ok(Box::new(DynamicNode::new(
            "passthrough",
            vec![PortDef::text("in")],
            vec![PortDef::text("out")],
            None,
            Arc::new(|mut inputs, _| {
                Box::pin(async move {
                    let val = inputs
                        .take_text("in")
                        .map_err(|_e| error_stack::Report::new(NodeError))?;
                    let mut out = PortValues::new();
                    out.insert(
                        "out".to_owned(),
                        PortValue::Single(ScalarValue::Text(val)),
                    );
                    Ok(out)
                })
            }),
        )))
    }
}

/// A factory that creates sink nodes.
struct SinkFactory;

impl NodeFactory for SinkFactory {
    fn create(
        &self,
        _config: serde_json::Value,
    ) -> Result<Box<dyn WorkflowNode>, error_stack::Report<nullslop_workflow::registry::RegistryError>>
    {
        Ok(Box::new(DynamicNode::new(
            "sink",
            vec![PortDef::text("in")],
            vec![],
            None,
            Arc::new(|_, _| Box::pin(async { Ok(PortValues::new()) })),
        )))
    }
}

#[expect(clippy::expect_used, reason = "example code")]
fn main() {
    let mut terminal = common::setup_terminal();

    // Build registry with factories.
    let mut registry = NodeRegistry::new();
    registry.register("source", Box::new(SourceFactory));
    registry.register("passthrough", Box::new(PassthroughFactory));
    registry.register("sink", Box::new(SinkFactory));

    // Build graph from registry entries.
    let graph = {
        let mut b = WorkflowGraphBuilder::new();
        b.add_node_from_registry(
            "source".to_owned(),
            &registry,
            "source",
            serde_json::json!({"output": "hello"}),
        )
        .expect("add source")
        .add_node_from_registry(
            "step1".to_owned(),
            &registry,
            "passthrough",
            serde_json::json!({}),
        )
        .expect("add step1")
        .add_node_from_registry(
            "step2".to_owned(),
            &registry,
            "passthrough",
            serde_json::json!({}),
        )
        .expect("add step2")
        .add_node_from_registry(
            "sink".to_owned(),
            &registry,
            "sink",
            serde_json::json!({}),
        )
        .expect("add sink");

        b.connect("source", "out", "step1", "in").expect("connect");
        b.connect("step1", "out", "step2", "in").expect("connect");
        b.connect("step2", "out", "sink", "in").expect("connect");
        b.build().expect("graph should build")
    };

    // Display with mixed statuses.
    let execution = WorkflowExecution::new(graph);
    execution.set_status("source", NodeStatus::Completed);
    execution.set_status("step1", NodeStatus::Completed);
    execution.set_status("step2", NodeStatus::Running);
    // sink stays Pending.
    let snapshot = execution.snapshot();
    let viewport = ViewportState::new();

    terminal
        .draw(|f| {
            let widget = WorkflowWidget::new(&snapshot, &viewport, 0);
            widget.render(f.area(), f.buffer_mut());
        })
        .expect("draw failed");

    thread::sleep(Duration::from_secs(5));
    common::restore_terminal(&mut terminal);
}
