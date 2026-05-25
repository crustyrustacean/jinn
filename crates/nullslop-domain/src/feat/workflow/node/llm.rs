//! LLM workflow node.
//!
//! [`LlmNode`] is a workflow node that sends a prompt to the LLM and returns
//! the response as a string output port.
//!
//! # Ports
//!
//! - Input `system` (optional, string) — system prompt override.
//! - Input `prompt` (optional, string) — template/context from upstream.
//! - Input `user` (required, string) — the user message body.
//! - Output `response` (string) — the final assistant response.
//!
//! If both `prompt` and `user` are connected, their values are concatenated
//! (`prompt` first, then `user`). If only `user` is connected, only the user
//! value is used. The `system` port overrides the node's configured system
//! prompt when connected.

use error_stack::Report;
use nullslop_workflow::node::{NodeContext, NodeError, WorkflowNode};
use nullslop_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};
use nullslop_workflow::tool_schema::ToolSchema;

/// A workflow node that calls the LLM.
///
/// # Configuration
///
/// - `system_prompt` — default system prompt. Overridden by the `system` input
///   port when connected.
/// - `provider_id` — optional provider ID. `None` uses the global default.
/// - `tool_schemas` — tool definitions available to the LLM during this call.
///
/// # Port design
///
/// The three input ports allow flexible wiring:
///
/// | Connected ports | User message content | System prompt |
/// |-----------------|---------------------|---------------|
/// | `user` only     | `user` value         | configured default |
/// | `prompt` + `user` | `prompt` + `user` | configured default |
/// | `system` + `user` | `user` value       | `system` value |
/// | all three       | `prompt` + `user`   | `system` value |
#[derive(Debug, Clone)]
pub struct LlmNode {
    /// Default system prompt. Overridden by the `system` input port when connected.
    system_prompt: Option<String>,
    /// Optional provider ID override. `None` = global default.
    provider_id: Option<String>,
    /// Tool definitions available to the LLM during this call.
    tool_schemas: Vec<ToolSchema>,
}

impl LlmNode {
    /// Create a new LLM node with the given default system prompt.
    #[must_use]
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: Some(system_prompt.into()),
            provider_id: None,
            tool_schemas: vec![],
        }
    }

    /// Create an LLM node with no default system prompt.
    #[must_use]
    pub fn without_system_prompt() -> Self {
        Self {
            system_prompt: None,
            provider_id: None,
            tool_schemas: vec![],
        }
    }

    /// Set a specific provider ID for this node.
    #[must_use]
    pub fn with_provider(mut self, provider_id: impl Into<String>) -> Self {
        self.provider_id = Some(provider_id.into());
        self
    }

    /// Add tool definitions for this node.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tool_schemas = tools;
        self
    }
}

#[async_trait::async_trait]
impl WorkflowNode for LlmNode {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait contract requires &str"
    )]
    fn name(&self) -> &str {
        "llm"
    }

    fn input_ports(&self) -> Vec<PortDef> {
        vec![
            PortDef::text("system").optional(),
            PortDef::text("prompt").optional(),
            PortDef::text("user"),
        ]
    }

    fn output_ports(&self) -> Vec<PortDef> {
        vec![PortDef::text("response")]
    }

    async fn execute(
        &self,
        mut inputs: PortValues,
        ctx: &dyn NodeContext,
    ) -> Result<PortValues, Report<NodeError>> {
        // Resolve system prompt: input port overrides configured default.
        let system_prompt = inputs
            .take_text("system")
            .ok()
            .or_else(|| self.system_prompt.clone());

        // Build user message from prompt + user inputs.
        let prompt_text = inputs.take_text("prompt").ok();
        let user_text =
            inputs
                .take_text("user")
                .map_err(|e: nullslop_workflow::port::PortError| {
                    Report::new(NodeError).attach(e.to_string())
                })?;

        let user_message = match prompt_text {
            Some(prompt) => format!("{prompt}\n{user_text}"),
            None => user_text,
        };

        let response = ctx
            .send_llm_request(
                &user_message,
                system_prompt.as_deref(),
                self.tool_schemas.clone(),
                self.provider_id.as_deref(),
            )
            .await?;

        let mut output = PortValues::new();
        output.insert(
            "response".to_owned(),
            PortValue::single(ScalarValue::Text(response)),
        );
        Ok(output)
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(self.clone())
    }
}
