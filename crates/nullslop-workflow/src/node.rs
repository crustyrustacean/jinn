//! Node trait and execution context.
//!
//! A [`WorkflowNode`] is the core unit of computation in a workflow graph.
//! Each node declares its input and output ports, then executes when all
//! input ports are satisfied.

use error_stack::Report;
use wherror::Error;

use crate::port::PortValues;

/// Execution context passed to nodes during execution.
///
/// Implemented by the host environment. For tests, use a simple unit struct.
/// For domain integration (Part 2), `DomainNodeContext` will provide access
/// to services, LLM requests, and the actor bus.
pub trait NodeContext: Send + Sync {}

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
///     fn input_ports(&self) -> Vec<PortDef> { vec![PortDef::string("input")] }
///     fn output_ports(&self) -> Vec<PortDef> { vec![PortDef::string("output")] }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::{PortDef, PortValue};

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
            vec![PortDef::string("in")]
        }

        fn output_ports(&self) -> Vec<PortDef> {
            vec![PortDef::string("out")]
        }

        async fn execute(
            &self,
            mut inputs: PortValues,
            _ctx: &dyn NodeContext,
        ) -> Result<PortValues, Report<NodeError>> {
            let value = inputs
                .take_string("in")
                .map_err(|_| Report::new(NodeError))?;
            let mut output = PortValues::new();
            output.insert("out", PortValue::String(value));
            Ok(output)
        }
    }

    #[tokio::test]
    async fn echo_node_returns_input_as_output() {
        // Given an echo node and a test context.
        let node = EchoNode;
        let ctx = TestContext;
        let mut inputs = PortValues::new();
        inputs.insert("in", PortValue::String("hello".to_owned()));

        // When executing the node.
        let result = node.execute(inputs, &ctx).await;

        // Then it returns the input as output.
        let outputs = result.expect("echo should succeed");
        assert_eq!(outputs.get_string("out").unwrap(), "hello");
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
