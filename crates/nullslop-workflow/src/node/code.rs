//! CodeNode — a closure-based workflow node.
//!
//! [`CodeNode`] wraps an async closure, making it easy to define custom
//! workflow nodes without implementing the full [`WorkflowNode`](crate::node::WorkflowNode) trait.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use error_stack::Report;

use crate::node::{NodeContext, NodeError, WorkflowNode};
use crate::port::{PortDef, PortValues};

/// The type of the closure stored in a [`CodeNode`].
type ExecuteFn = Arc<
    dyn Fn(
            PortValues,
            &dyn NodeContext,
        ) -> Pin<Box<dyn Future<Output = Result<PortValues, Report<NodeError>>> + Send>>
        + Send
        + Sync,
>;

/// A node defined by an async closure.
///
/// The closure receives [`PortValues`] and `&dyn `[`NodeContext`],
/// and returns `Result<`[`PortValues`]`, `[`Report`]`<`[`NodeError`]`>>`.
///
/// The closure is wrapped in `Arc` so that `CodeNode` is clonable — the
/// execution engine clones nodes when spawning tasks.
///
/// # Examples
///
/// ```rust,ignore
/// use nullslop_workflow::node::CodeNode;
/// use nullslop_workflow::port::{PortDef, PortValues, PortValue, ScalarValue};
///
/// let node = CodeNode::new(
///     "uppercase",
///     vec![PortDef::text("in")],
///     vec![PortDef::text("out")],
///     |mut inputs, _ctx| {
///         Box::pin(async move {
///             let val = inputs.take_text("in").map_err(|_| Report::new(NodeError))?;
///             let mut out = PortValues::new();
///             out.insert("out".to_owned(), PortValue::Single(ScalarValue::Text(val.to_uppercase())));
///             Ok(out)
///         })
///     },
/// );
/// ```
pub struct CodeNode {
    /// Human-readable name for debugging.
    name: String,
    /// Declared input ports.
    input_ports: Vec<PortDef>,
    /// Declared output ports.
    output_ports: Vec<PortDef>,
    /// The async closure that implements the node's logic, wrapped in Arc for cloning.
    execute_fn: ExecuteFn,
}

impl CodeNode {
    /// Creates a new `CodeNode`.
    ///
    /// The closure must be `Send + Sync + 'static` and return a pinned,
    /// boxed, sendable future.
    #[must_use]
    pub fn new<F, Fut>(
        name: String,
        inputs: Vec<PortDef>,
        outputs: Vec<PortDef>,
        execute_fn: F,
    ) -> Self
    where
        F: Fn(PortValues, &dyn NodeContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<PortValues, Report<NodeError>>> + Send + 'static,
    {
        Self {
            name,
            input_ports: inputs,
            output_ports: outputs,
            execute_fn: Arc::new(move |inputs, ctx| Box::pin(execute_fn(inputs, ctx))),
        }
    }
}

#[async_trait::async_trait]
impl WorkflowNode for CodeNode {
    fn name(&self) -> &'static str {
        // Intentional leak for 'static name.
        Box::leak(self.name.clone().into_boxed_str())
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
            name: self.name.clone(),
            input_ports: self.input_ports.clone(),
            output_ports: self.output_ports.clone(),
            execute_fn: Arc::clone(&self.execute_fn),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::{PortValue, ScalarValue};
    use std::sync::Arc;

    /// A minimal NodeContext for tests.
    struct TestContext;
    impl NodeContext for TestContext {}

    #[tokio::test]
    async fn code_node_executes_closure() {
        // Given a CodeNode that uppercases its input.
        let ctx = Arc::new(TestContext);
        let node = CodeNode::new(
            "uppercase".to_owned(),
            vec![PortDef::text("in")],
            vec![PortDef::text("out")],
            |mut inputs, _ctx| {
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
            },
        );

        // When executing the node.
        let mut inputs = PortValues::new();
        inputs.insert(
            "in".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        let result = node.execute(inputs, &*ctx).await;

        // Then it returns the uppercased value.
        #[expect(clippy::expect_used, reason = "test assertion")]
        let outputs = result.expect("code node should succeed");
        assert_eq!(outputs.get_text("out").unwrap(), "HELLO");
    }

    #[tokio::test]
    async fn code_node_can_fail() {
        // Given a CodeNode that always fails.
        let ctx = Arc::new(TestContext);
        let node = CodeNode::new(
            "fail".to_owned(),
            vec![PortDef::text("in")],
            vec![PortDef::text("out")],
            |_inputs, _ctx| Box::pin(async move { Err(Report::new(NodeError)) }),
        );

        // When executing the node.
        let mut inputs = PortValues::new();
        inputs.insert(
            "in".to_owned(),
            PortValue::Single(ScalarValue::Text("data".to_owned())),
        );
        let result = node.execute(inputs, &*ctx).await;

        // Then it returns an error.
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn code_node_is_clonable() {
        // Given a CodeNode.
        let node = CodeNode::new(
            "echo".to_owned(),
            vec![PortDef::text("in")],
            vec![PortDef::text("out")],
            |mut inputs, _ctx| {
                Box::pin(async move {
                    let val = inputs
                        .take_text("in")
                        .map_err(|_e| Report::new(NodeError))?;
                    let mut out = PortValues::new();
                    out.insert("out".to_owned(), PortValue::Single(ScalarValue::Text(val)));
                    Ok(out)
                })
            },
        );

        // When cloning via clone_box.
        let cloned = node.clone_box();

        // Then the cloned node works independently.
        let mut inputs = PortValues::new();
        inputs.insert(
            "in".to_owned(),
            PortValue::Single(ScalarValue::Text("test".to_owned())),
        );
        let ctx = Arc::new(TestContext);
        let result = cloned.execute(inputs, &*ctx).await;
        #[expect(clippy::expect_used, reason = "test assertion")]
        let outputs = result.expect("cloned code node should succeed");
        assert_eq!(outputs.get_text("out").unwrap(), "test");
    }
}
