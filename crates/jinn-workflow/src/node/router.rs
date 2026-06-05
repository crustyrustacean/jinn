//! RouterNode - conditional branching node for workflow graphs.
//!
//! [`RouterNode`] receives a text input, matches it against regex patterns to
//! select one output port, and populates only that port. Downstream nodes on
//! unpopulated branches are handled by the engine's deadlock detection - they
//! are marked `Skipped`.
//!
//! # Example
//!
//! See the tests in this crate for full construction examples.

use error_stack::Report;
use regex::Regex;
use serde_json::json;

use crate::node::{NodeContext, NodeError, WorkflowNode};
use crate::port::{PortDef, PortValue, PortValues, ScalarValue};

/// A conditional branching node that routes input to exactly one output port.
///
/// The router matches the input text against regex patterns. The first matching
/// route wins. If no route matches and a default port is configured, the default
/// is used. Otherwise, the node returns an error.
///
/// Only the matching output port is populated in the returned [`PortValues`].
/// Unpopulated ports cause downstream nodes to be marked `Skipped` by the
/// engine's deadlock detection.
pub struct RouterNode {
    /// Human-readable name for debugging.
    name: String,
    /// The input port definition.
    input_port: PortDef,
    /// All declared output port definitions.
    output_ports: Vec<PortDef>,
    /// Compiled routing rules: (port_name, regex).
    routes: Vec<(String, Regex)>,
    /// Optional fallback port when no route matches.
    default_port: Option<String>,
}

impl RouterNode {
    /// Creates a new `RouterNode`.
    ///
    /// # Arguments
    ///
    /// * `name` - Human-readable name for this node.
    /// * `input_port` - The single input port definition.
    /// * `output_ports` - All possible output port definitions.
    ///
    /// # Panics
    ///
    /// Panics if `output_ports` is empty.
    #[must_use]
    pub fn new(name: String, input_port: PortDef, output_ports: Vec<PortDef>) -> Self {
        assert!(
            !output_ports.is_empty(),
            "RouterNode must have at least one output port"
        );
        Self {
            name,
            input_port,
            output_ports,
            routes: Vec::new(),
            default_port: None,
        }
    }

    /// Adds a routing rule: if the input matches `pattern`, route to `port_name`.
    ///
    /// Routes are evaluated in insertion order; the first match wins.
    ///
    /// # Panics
    ///
    /// Panics if `port_name` is not in the declared `output_ports`.
    /// Panics if the regex pattern is invalid.
    #[expect(clippy::panic, reason = "programming error: invalid regex pattern")]
    #[must_use]
    pub fn with_route(mut self, port_name: String, pattern: &str) -> Self {
        assert!(
            self.output_ports.iter().any(|p| p.name == port_name),
            "route port '{port_name}' not found in output_ports"
        );
        let regex =
            Regex::new(pattern).unwrap_or_else(|e| panic!("invalid regex '{pattern}': {e}"));
        self.routes.push((port_name, regex));
        self
    }

    /// Sets the default output port used when no route matches.
    ///
    /// # Panics
    ///
    /// Panics if `port_name` is not in the declared `output_ports`.
    #[must_use]
    pub fn with_default(mut self, port_name: String) -> Self {
        assert!(
            self.output_ports.iter().any(|p| p.name == port_name),
            "default port '{port_name}' not found in output_ports"
        );
        self.default_port = Some(port_name);
        self
    }
}

#[async_trait::async_trait]
impl WorkflowNode for RouterNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn input_ports(&self) -> Vec<PortDef> {
        vec![self.input_port.clone()]
    }

    fn output_ports(&self) -> Vec<PortDef> {
        self.output_ports.clone()
    }

    async fn execute(
        &self,
        mut inputs: PortValues,
        _ctx: &dyn NodeContext,
    ) -> Result<PortValues, Report<NodeError>> {
        let input_value = inputs
            .take_text(&self.input_port.name)
            .map_err(|_e| Report::new(NodeError))?;

        // Try each route in order; first match wins.
        for (port_name, regex) in &self.routes {
            if regex.is_match(&input_value) {
                let mut out = PortValues::new();
                out.insert(
                    port_name.clone(),
                    PortValue::Single(ScalarValue::Text(input_value)),
                );
                return Ok(out);
            }
        }

        // Fallback to default port.
        if let Some(default) = &self.default_port {
            let mut out = PortValues::new();
            out.insert(
                default.clone(),
                PortValue::Single(ScalarValue::Text(input_value)),
            );
            return Ok(out);
        }

        // No route matched and no default - error.
        Err(Report::new(NodeError))
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(Self {
            name: self.name.clone(),
            input_port: self.input_port.clone(),
            output_ports: self.output_ports.clone(),
            routes: self.routes.clone(),
            default_port: self.default_port.clone(),
        })
    }

    fn config(&self) -> Option<serde_json::Value> {
        let routes: Vec<serde_json::Value> = self
            .routes
            .iter()
            .map(|(name, regex)| {
                json!({
                    "port": name,
                    "pattern": regex.as_str(),
                })
            })
            .collect();

        Some(json!({
            "type": "router",
            "routes": routes,
            "default": self.default_port,
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::needless_raw_string_hashes,
        clippy::needless_raw_strings,
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::get_unwrap,
        reason = "test code"
    )]
    use super::*;
    use std::sync::Arc;

    /// A minimal NodeContext for tests.
    struct TestContext;
    impl NodeContext for TestContext {}

    fn binary_router() -> RouterNode {
        RouterNode::new(
            "router".to_owned(),
            PortDef::text("in"),
            vec![PortDef::text("yes"), PortDef::text("no")],
        )
        .with_route("yes".to_owned(), r"(?i)^yes")
        .with_route("no".to_owned(), r"(?i)^no")
    }

    fn make_text_input(value: &str) -> PortValues {
        let mut inputs = PortValues::new();
        inputs.insert(
            "in".to_owned(),
            PortValue::Single(ScalarValue::Text(value.to_owned())),
        );
        inputs
    }

    #[tokio::test]
    async fn binary_routing_yes() {
        let router = binary_router();
        let ctx = Arc::new(TestContext);
        let result = router.execute(make_text_input("YES"), &*ctx).await;

        #[expect(clippy::expect_used, reason = "test assertion")]
        let outputs = result.expect("router should succeed");
        assert!(outputs.contains("yes"), "'yes' port must be populated");
        assert_eq!(outputs.get_text("yes").unwrap(), "YES");
        assert!(!outputs.contains("no"), "'no' port must not be populated");
    }

    #[tokio::test]
    async fn binary_routing_no() {
        let router = binary_router();
        let ctx = Arc::new(TestContext);
        let result = router.execute(make_text_input("NO"), &*ctx).await;

        #[expect(clippy::expect_used, reason = "test assertion")]
        let outputs = result.expect("router should succeed");
        assert!(outputs.contains("no"), "'no' port must be populated");
        assert_eq!(outputs.get_text("no").unwrap(), "NO");
        assert!(!outputs.contains("yes"), "'yes' port must not be populated");
    }

    #[tokio::test]
    async fn ternary_routing() {
        let router = RouterNode::new(
            "tri-router".to_owned(),
            PortDef::text("in"),
            vec![
                PortDef::text("low"),
                PortDef::text("medium"),
                PortDef::text("high"),
            ],
        )
        .with_route("low".to_owned(), r"(?i)^low")
        .with_route("medium".to_owned(), r"(?i)^medium")
        .with_route("high".to_owned(), r"(?i)^high");

        let ctx = Arc::new(TestContext);
        let result = router.execute(make_text_input("medium"), &*ctx).await;

        #[expect(clippy::expect_used, reason = "test assertion")]
        let outputs = result.expect("router should succeed");
        assert!(
            outputs.contains("medium"),
            "'medium' port must be populated"
        );
        assert!(!outputs.contains("low"), "'low' port must not be populated");
        assert!(
            !outputs.contains("high"),
            "'high' port must not be populated"
        );
    }

    #[tokio::test]
    async fn no_match_no_default_returns_error() {
        let router = binary_router();
        let ctx = Arc::new(TestContext);
        let result = router.execute(make_text_input("maybe"), &*ctx).await;
        assert!(result.is_err(), "unmatched input must return error");
    }

    #[tokio::test]
    async fn default_port_fallback() {
        let router = RouterNode::new(
            "router".to_owned(),
            PortDef::text("in"),
            vec![
                PortDef::text("yes"),
                PortDef::text("no"),
                PortDef::text("unknown"),
            ],
        )
        .with_route("yes".to_owned(), r"(?i)^yes")
        .with_route("no".to_owned(), r"(?i)^no")
        .with_default("unknown".to_owned());

        let ctx = Arc::new(TestContext);
        let result = router.execute(make_text_input("maybe"), &*ctx).await;

        #[expect(clippy::expect_used, reason = "test assertion")]
        let outputs = result.expect("router should succeed with default");
        assert!(
            outputs.contains("unknown"),
            "'unknown' port must be populated"
        );
        assert_eq!(outputs.get_text("unknown").unwrap(), "maybe");
    }

    #[test]
    #[should_panic(expected = "not found in output_ports")]
    fn route_port_not_in_output_ports_panics() {
        let _ = RouterNode::new(
            "router".to_owned(),
            PortDef::text("in"),
            vec![PortDef::text("yes"), PortDef::text("no")],
        )
        .with_route("invalid_port".to_owned(), r".*");
    }

    #[test]
    #[should_panic(expected = "not found in output_ports")]
    fn default_port_not_in_output_ports_panics() {
        let _ = RouterNode::new(
            "router".to_owned(),
            PortDef::text("in"),
            vec![PortDef::text("yes"), PortDef::text("no")],
        )
        .with_default("invalid_port".to_owned());
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "end-to-end router test exercises every branch of the routing logic"
    )]
    async fn e2e_with_engine_router_skips_branch() {
        use crate::engine::{self, NodeStatus};
        use crate::graph::WorkflowGraphBuilder;

        let ctx = Arc::new(TestContext);

        // Build graph: source → router → (yes_branch, no_branch)
        let mut builder = WorkflowGraphBuilder::new();
        builder.add_node(
            "source".to_owned(),
            Box::new(crate::node::code::CodeNode::new(
                "source".to_owned(),
                vec![],
                vec![PortDef::text("out")],
                |_inputs, _ctx| {
                    Box::pin(async {
                        let mut out = PortValues::new();
                        out.insert(
                            "out".to_owned(),
                            PortValue::Single(ScalarValue::Text("YES".to_owned())),
                        );
                        Ok(out)
                    })
                },
            )),
        );
        builder.add_node(
            "router".to_owned(),
            Box::new(
                RouterNode::new(
                    "router".to_owned(),
                    PortDef::text("in"),
                    vec![PortDef::text("yes"), PortDef::text("no")],
                )
                .with_route("yes".to_owned(), r"(?i)^yes")
                .with_route("no".to_owned(), r"(?i)^no"),
            ),
        );
        builder.add_node(
            "yes_branch".to_owned(),
            Box::new(crate::node::code::CodeNode::new(
                "yes_branch".to_owned(),
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
            )),
        );
        builder.add_node(
            "no_branch".to_owned(),
            Box::new(crate::node::code::CodeNode::new(
                "no_branch".to_owned(),
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
                            PortValue::Single(ScalarValue::Text(val.to_lowercase())),
                        );
                        Ok(out)
                    })
                },
            )),
        );

        builder
            .connect("source", "out", "router", "in")
            .expect("src→router");
        builder
            .connect("router", "yes", "yes_branch", "in")
            .expect("router→yes");
        builder
            .connect("router", "no", "no_branch", "in")
            .expect("router→no");

        let graph = builder.build().expect("build");
        let execution = Arc::new(crate::execution::WorkflowExecution::new(graph));

        let result = engine::execute(execution, ctx).await.expect("execute");

        // yes_branch should have completed with the routed value.
        assert_eq!(
            result.statuses.get("yes_branch").unwrap(),
            &NodeStatus::Completed
        );
        assert_eq!(
            result
                .outputs
                .get("yes_branch")
                .unwrap()
                .get_text("out")
                .unwrap(),
            "YES"
        );

        // no_branch should be skipped (deadlocked).
        assert_eq!(
            result.statuses.get("no_branch").unwrap(),
            &NodeStatus::Skipped
        );
    }

    #[test]
    fn config_returns_routing_configuration() {
        let router = RouterNode::new(
            "router".to_owned(),
            PortDef::text("in"),
            vec![PortDef::text("yes"), PortDef::text("no")],
        )
        .with_route("yes".to_owned(), r"(?i)^yes")
        .with_route("no".to_owned(), r"(?i)^no")
        .with_default("no".to_owned());

        let config = router.config().expect("config should return Some");
        assert_eq!(config["type"], "router");
        assert_eq!(config["routes"].as_array().unwrap().len(), 2);
        assert_eq!(config["default"], "no");
    }

    #[test]
    fn clone_box_produces_working_copy() {
        let router = binary_router();
        let cloned = router.clone_box();

        assert_eq!(cloned.name(), "router");
        assert_eq!(cloned.output_ports().len(), 2);
    }
}
