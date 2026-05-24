//! Node trait, execution context, and built-in node types.
//!
//! A [`WorkflowNode`] is the core unit of computation in a workflow graph.
//! Each node declares its input and output ports, then executes when all
//! input ports are satisfied.
//!
//! # Built-in nodes
//!
//! - [`CodeNode`] — wraps an async closure for quick custom logic.
//! - [`DelayNode`] — sleeps for a configured duration, then passes inputs through.

use std::future::Future;
use std::pin::Pin;

use error_stack::Report;
use wherror::Error;

use crate::port::PortValues;
use crate::tool_schema::ToolSchema;

pub mod code;
pub mod delay;
pub mod dynamic;

pub use code::CodeNode;
pub use delay::DelayNode;
pub use dynamic::DynamicNode;

/// Execution context passed to nodes during execution.
///
/// Implemented by the host environment. For tests, use a simple unit struct.
/// For domain integration, `DomainNodeContext` provides access
/// to services, LLM requests, and the actor bus.
///
/// Default methods are no-ops or return errors. Override them in the
/// host environment's implementation.
pub trait NodeContext: Send + Sync {
    /// Send an LLM request through the session pipeline and await the full response.
    ///
    /// Creates a new workflow session, stores the provided overrides, enqueues a user
    /// message, waits for `SessionPhaseChanged(Idle)`, and extracts the final assistant
    /// response.
    ///
    /// - `user_prompt` — the user message text.
    /// - `system_prompt` — optional system prompt override.
    /// - `tool_schemas` — tool definitions for this request.
    /// - `provider_id` — optional provider ID override; `None` uses global default.
    ///
    /// Default: returns an error (no LLM capability).
    /// Override in `DomainNodeContext` to route through the actor bus.
    fn send_llm_request<'a>(
        &'a self,
        user_prompt: &str,
        system_prompt: Option<&str>,
        tool_schemas: Vec<ToolSchema>,
        provider_id: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, Report<NodeError>>> + Send + 'a>> {
        let _ = (user_prompt, system_prompt, tool_schemas, provider_id);
        Box::pin(async { Err(Report::new(NodeError)) })
    }

    /// Called by the engine before executing a node.
    ///
    /// Domain implementations can use this to record node identity
    /// for session tracking and other cross-cutting concerns.
    fn set_node_name(&self, _name: &str) {}

    /// Called by the engine after node execution completes.
    fn clear_node_name(&self) {}
}

/// Error type for node execution failures.
///
/// Opaque for MVP — nodes attach context via [`Report::attach`]. Will evolve
/// to an enum when structured error variants are needed (timeout, cancelled, etc.).
#[derive(Debug, Error)]
#[error(debug)]
pub struct NodeError;

/// A unit of computation in a workflow graph.
///
/// Nodes declare their input and output ports via [`PortDef`](crate::port::PortDef),
/// then execute when all input ports are satisfied. The engine guarantees that
/// `inputs` contains exactly the ports declared by `input_ports()`, each with
/// the correct type.
///
/// # Implementing a node
///
/// ```rust,ignore
/// struct MyNode;
///
/// #[async_trait::async_trait]
/// impl WorkflowNode for MyNode {
///     fn name(&self) -> &str { "my-node" }
///     fn input_ports(&self) -> Vec<PortDef> { vec![PortDef::text("input")] }
///     fn output_ports(&self) -> Vec<PortDef> { vec![PortDef::text("output")] }
///
///     async fn execute(
///         &self,
///         mut inputs: PortValues,
///         _ctx: &dyn NodeContext,
///     ) -> Result<PortValues, NodeError> {
///         let input = inputs.take_string("input").change_context(NodeError)?;
///         let output = input.to_uppercase();
///         Ok(PortValues::from([("output".to_owned(), PortValue::String(output))]))
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait WorkflowNode: Send + Sync {
    /// Human-readable name for this node type (for debugging and UI).
    fn name(&self) -> &str;

    /// Declare the input ports this node accepts.
    fn input_ports(&self) -> Vec<crate::port::PortDef>;

    /// Declare the output ports this node produces.
    fn output_ports(&self) -> Vec<crate::port::PortDef>;

    /// Clones this node into a boxed trait object.
    ///
    /// Used by the execution engine to move nodes into spawned tasks.
    fn clone_box(&self) -> Box<dyn WorkflowNode>;

    /// Execute the node.
    ///
    /// `inputs` is guaranteed to contain exactly the ports declared by
    /// [`input_ports`](Self::input_ports), each with the correct
    /// [`PortType`](crate::port::PortType).
    ///
    /// Returns a [`PortValues`] containing values for every port declared by
    /// [`output_ports`](Self::output_ports).
    ///
    /// # Errors
    ///
    /// Returns a [`Report<NodeError>`] if execution fails.
    async fn execute(
        &self,
        inputs: PortValues,
        ctx: &dyn NodeContext,
    ) -> Result<PortValues, Report<NodeError>>;

    /// Returns the node's static configuration for display and inspection.
    ///
    /// Override to expose node parameters (URLs, file paths, prompts, etc.).
    /// Captured once at construction time; never changes during execution.
    /// Returns `None` by default.
    fn config(&self) -> Option<serde_json::Value> {
        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unnecessary_literal_bound, reason = "test code")]
    use super::*;
    use crate::port::{PortDef, PortValue, ScalarValue};

    /// A minimal NodeContext for tests.
    struct TestContext;

    impl NodeContext for TestContext {}

    /// A trivial node for testing the trait compiles and works.
    struct EchoNode;

    #[async_trait::async_trait]
    impl WorkflowNode for EchoNode {
        fn name(&self) -> &str {
            "echo"
        }

        fn input_ports(&self) -> Vec<PortDef> {
            vec![PortDef::text("in")]
        }

        fn output_ports(&self) -> Vec<PortDef> {
            vec![PortDef::text("out")]
        }

        async fn execute(
            &self,
            mut inputs: PortValues,
            _ctx: &dyn NodeContext,
        ) -> Result<PortValues, Report<NodeError>> {
            let value = inputs
                .take_text("in")
                .map_err(|_port_err| Report::new(NodeError))?;
            let mut output = PortValues::new();
            output.insert("out".to_owned(), PortValue::Single(ScalarValue::Text(value)));
            Ok(output)
        }

        fn clone_box(&self) -> Box<dyn WorkflowNode> {
            Box::new(EchoNode)
        }
    }

    #[tokio::test]
    async fn echo_node_returns_input_as_output() {
        // Given an echo node and a test context.
        let node = EchoNode;
        let ctx = TestContext;
        let mut inputs = PortValues::new();
        inputs.insert("in".to_owned(), PortValue::Single(ScalarValue::Text("hello".to_owned())));

        // When executing the node.
        let result = node.execute(inputs, &ctx).await;

        // Then it returns the input as output.
        #[expect(clippy::expect_used, reason = "test assertion")]
        let outputs = result.expect("echo should succeed");
        assert_eq!(outputs.get_text("out").unwrap(), "hello");
    }

    #[test]
    fn node_error_is_debug() {
        // Given a NodeError.
        let err = NodeError;

        // When formatting as debug.
        // Then it produces a non-empty string.
        assert!(!format!("{err:?}").is_empty());
    }
}
