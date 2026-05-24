//! Dynamic node type with runtime port declarations.
//!
//! [`DynamicNode`] is a node whose ports and config are defined at runtime
//! from data, rather than at compile time. This enables data-driven graph
//! construction where node types come from configuration files or scripts.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use error_stack::Report;

use crate::node::{NodeContext, NodeError, WorkflowNode};
use crate::port::{PortDef, PortValues, PortValue, ScalarValue};

/// Type alias for the dynamic execute function.
type DynamicExecuteFn = Arc<
    dyn Fn(
            PortValues,
            &dyn NodeContext,
        ) -> Pin<Box<dyn Future<Output = Result<PortValues, Report<NodeError>>> + Send>>
        + Send
        + Sync,
>;

/// A node whose ports and config are defined at runtime from data.
///
/// Unlike [`CodeNode`](super::code::CodeNode) or [`DelayNode`](super::delay::DelayNode),
/// which have fixed port layouts, `DynamicNode` takes port definitions as constructor
/// arguments. This enables data-driven graph construction where node types come
/// from configuration files or scripts.
///
/// The execute function is provided as an `Arc<dyn Fn>` closure, the same pattern
/// used by `CodeNode`.
pub struct DynamicNode {
    /// The node's name, leaked to &'static str.
    static_name: &'static str,
    /// Input port definitions.
    input_ports: Vec<PortDef>,
    /// Output port definitions.
    output_ports: Vec<PortDef>,
    /// Optional configuration.
    config: Option<serde_json::Value>,
    /// The execute function closure.
    execute_fn: DynamicExecuteFn,
}

impl DynamicNode {
    /// Creates a new dynamic node.
    ///
    /// The `name` is leaked once at construction to satisfy the `&'static str`
    /// return type of [`WorkflowNode::name`]. This is the same pattern as `CodeNode`.
    pub fn new<S: Into<String>>(
        name: S,
        input_ports: Vec<PortDef>,
        output_ports: Vec<PortDef>,
        config: Option<serde_json::Value>,
        execute_fn: DynamicExecuteFn,
    ) -> Self {
        let static_name = Box::leak(name.into().into_boxed_str());
        Self {
            static_name,
            input_ports,
            output_ports,
            config,
            execute_fn,
        }
    }

    /// Creates a passthrough dynamic node that copies input "in" to output "out".
    pub fn passthrough<S: Into<String>>(name: S) -> Self {
        Self::new(
            name,
            vec![PortDef::text("in")],
            vec![PortDef::text("out")],
            None,
            Arc::new(|mut inputs, _ctx| {
                Box::pin(async move {
                    let val = inputs
                        .take_text("in")
                        .map_err(|_e| Report::new(NodeError))?;
                    let mut out = PortValues::new();
                    out.insert("out".to_owned(), PortValue::Single(ScalarValue::Text(val)));
                    Ok(out)
                })
            }),
        )
    }
}

#[async_trait]
impl WorkflowNode for DynamicNode {
    fn name(&self) -> &'static str {
        self.static_name
    }

    fn input_ports(&self) -> Vec<PortDef> {
        self.input_ports.clone()
    }

    fn output_ports(&self) -> Vec<PortDef> {
        self.output_ports.clone()
    }

    async fn execute(
        &self,
        inputs: PortValues,
        ctx: &dyn NodeContext,
    ) -> Result<PortValues, Report<NodeError>> {
        (self.execute_fn)(inputs, ctx).await
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(Self {
            static_name: self.static_name,
            input_ports: self.input_ports.clone(),
            output_ports: self.output_ports.clone(),
            config: self.config.clone(),
            execute_fn: self.execute_fn.clone(),
        })
    }

    fn config(&self) -> Option<serde_json::Value> {
        self.config.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::{PortValue, ScalarValue};

    /// A minimal NodeContext for tests.
    struct TestContext;
    impl NodeContext for TestContext {}

    #[test]
    fn reports_correct_ports() {
        // Given a DynamicNode with specific ports.
        let node = DynamicNode::new(
            "mynode",
            vec![PortDef::text("input_a"), PortDef::text("input_b")],
            vec![PortDef::text("output")],
            None,
            Arc::new(|_, _| Box::pin(async { Ok(PortValues::new()) })),
        );

        // Then input_ports and output_ports return the declared ports.
        assert_eq!(node.input_ports().len(), 2);
        assert_eq!(node.input_ports().first().map(|p| p.name.as_str()), Some("input_a"));
        assert_eq!(node.input_ports().get(1).map(|p| p.name.as_str()), Some("input_b"));
        assert_eq!(node.output_ports().len(), 1);
        assert_eq!(node.output_ports().first().map(|p| p.name.as_str()), Some("output"));
    }

    #[test]
    fn returns_config() {
        // Given a DynamicNode with config.
        let config = serde_json::json!({"key": "value"});
        let node = DynamicNode::new(
            "configured",
            vec![],
            vec![],
            Some(config.clone()),
            Arc::new(|_, _| Box::pin(async { Ok(PortValues::new()) })),
        );

        // Then config() returns the provided config.
        assert_eq!(node.config(), Some(config));
    }

    #[test]
    fn no_config_returns_none() {
        let node = DynamicNode::new(
            "noconfig",
            vec![],
            vec![],
            None,
            Arc::new(|_, _| Box::pin(async { Ok(PortValues::new()) })),
        );
        assert_eq!(node.config(), None);
    }

    #[test]
    fn clone_box_produces_equal_node() {
        let node = DynamicNode::passthrough("clone_test");
        let cloned = node.clone_box();

        assert_eq!(cloned.name(), node.name());
        assert_eq!(cloned.input_ports(), node.input_ports());
        assert_eq!(cloned.output_ports(), node.output_ports());
    }

    #[tokio::test]
    async fn passthrough_copies_input_to_output() {
        let node = DynamicNode::passthrough("passthrough_test");
        let mut inputs = PortValues::new();
        inputs.insert(
            "in".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );

        let result = node.execute(inputs, &TestContext).await;
        #[expect(clippy::expect_used, reason = "test assertion")]
        let outputs = result.expect("passthrough should succeed");

        assert_eq!(outputs.get_text("out").unwrap(), "hello");
    }

    #[test]
    fn name_returns_static_str() {
        let node = DynamicNode::passthrough("my_dynamic_node");
        assert_eq!(node.name(), "my_dynamic_node");
    }

    // Compile-time check: DynamicNode is Send + Sync.
    const _: () = {
        fn assert_send_sync<T: Send + Sync>() {}
        fn check() {
            assert_send_sync::<DynamicNode>();
        }
    };
}
