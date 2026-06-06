//! DelayNode - a node that delays before passing inputs through.
//!
//! [`DelayNode`] is useful for testing concurrent execution and async behavior.
//! All declared input ports are mirrored as output ports with matching names and types.

use std::time::Duration;

use error_stack::Report;

use crate::node::{NodeContext, NodeError, WorkflowNode};
use crate::port::{PortDef, PortValues};

/// A node that delays for a configured duration before passing
/// inputs through to matching output ports.
///
/// Input ports and output ports are 1:1 with matching names and types.
/// The node sleeps for the configured duration, then copies each input
/// value to the corresponding output port.
pub struct DelayNode {
    /// How long to sleep before producing outputs.
    duration: Duration,
    /// Port definitions - used for both input and output ports.
    ports: Vec<PortDef>,
}

impl DelayNode {
    /// Creates a new `DelayNode`.
    ///
    /// The `ports` define both input and output ports. Each input port
    /// is mirrored as an output port with the same name and type.
    #[must_use]
    pub fn new(duration: Duration, ports: Vec<PortDef>) -> Self {
        Self { duration, ports }
    }

    /// Convenience: creates a passthrough delay node with a single string port.
    #[must_use]
    pub fn passthrough(duration: Duration) -> Self {
        Self::new(duration, vec![PortDef::text("in")])
    }
}

#[async_trait::async_trait]
impl WorkflowNode for DelayNode {
    #[expect(
        clippy::unnecessary_literal_bound,
        reason = "trait contract requires &str"
    )]
    fn name(&self) -> &str {
        "delay"
    }

    fn input_ports(&self) -> Vec<PortDef> {
        self.ports.clone()
    }

    fn output_ports(&self) -> Vec<PortDef> {
        self.ports.clone()
    }

    async fn execute(
        &self,
        inputs: PortValues,
        _ctx: &dyn NodeContext,
    ) -> Result<PortValues, Report<NodeError>> {
        tokio::time::sleep(self.duration).await;
        // Copy all input values to output ports with matching names.
        let mut outputs = PortValues::new();
        for port_def in &self.ports {
            if let Some(value) = inputs.get(&port_def.name).cloned() {
                outputs.insert(port_def.name.clone(), value);
            }
        }
        Ok(outputs)
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(Self {
            duration: self.duration,
            ports: self.ports.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing, reason = "test code")]
    #![expect(clippy::expect_used, reason = "test code")]
    use super::*;
    use crate::port::{PortValue, ScalarValue};
    use std::time::Instant;

    #[tokio::test]
    async fn delay_node_delays_by_configured_duration() {
        // Given a DelayNode with a 100ms delay.
        let node = DelayNode::passthrough(Duration::from_millis(100));

        // When executing the node.
        let mut inputs = PortValues::new();
        inputs.insert(
            "in".to_owned(),
            PortValue::Single(ScalarValue::Text("data".to_owned())),
        );
        let start = Instant::now();
        let result = node.execute(inputs, &test_ctx()).await;

        // Then it delays by approximately the configured duration.
        let elapsed = start.elapsed();
        let outputs = result.expect("delay node should succeed");
        assert_eq!(outputs.get_text("in").unwrap(), "data");
        assert!(
            elapsed >= Duration::from_millis(80),
            "delay was too short: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn delay_node_passes_all_inputs_through() {
        // Given a DelayNode with two ports.
        let node = DelayNode::new(
            Duration::from_millis(10),
            vec![PortDef::text("text"), PortDef::text("label")],
        );

        // When executing with values for both ports.
        let mut inputs = PortValues::new();
        inputs.insert(
            "text".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        inputs.insert(
            "label".to_owned(),
            PortValue::Single(ScalarValue::Text("greeting".to_owned())),
        );
        let result = node.execute(inputs, &test_ctx()).await;

        // Then both values are passed through.
        let outputs = result.expect("delay node should succeed");
        assert_eq!(outputs.get_text("text").unwrap(), "hello");
        assert_eq!(outputs.get_text("label").unwrap(), "greeting");
    }

    /// A minimal NodeContext for tests.
    struct TestContext;
    impl NodeContext for TestContext {}

    fn test_ctx() -> TestContext {
        TestContext
    }

    // Kills: DelayNode::input_ports -> vec![]
    #[test]
    fn delay_node_input_ports_returns_actual_ports() {
        let node = DelayNode::passthrough(Duration::from_millis(10));
        let ports = node.input_ports();
        assert_eq!(ports.len(), 1, "must return 1 port, not empty");
        assert_eq!(ports[0].name, "in");
    }

    // Kills: DelayNode::output_ports -> vec![]
    #[test]
    fn delay_node_output_ports_returns_actual_ports() {
        let node = DelayNode::passthrough(Duration::from_millis(10));
        let ports = node.output_ports();
        assert_eq!(ports.len(), 1, "must return 1 port, not empty");
        assert_eq!(ports[0].name, "in");
    }
}
