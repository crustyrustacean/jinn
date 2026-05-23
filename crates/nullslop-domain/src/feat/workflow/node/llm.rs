//! LLM workflow node.
//!
//! [`LlmNode`] is a workflow node that sends a prompt to the LLM and returns
//! the response as a string output port.
//!
//! Two modes:
//! - **Source mode** ([`LlmNode::source`]) — zero input ports, embeds the user prompt.
//!   Used as the entry point of a workflow graph.
//! - **Internal mode** ([`LlmNode::new`]) — declares a `prompt` input port,
//!   receives data from upstream nodes.

use error_stack::Report;
use nullslop_workflow::node::{NodeContext, NodeError, WorkflowNode};
use nullslop_workflow::port::{PortDef, PortValue, PortValues};

/// A workflow node that calls the LLM.
///
/// # Source mode
///
/// Source nodes have zero input ports and embed the user prompt at construction
/// time. They are the entry points of a workflow graph.
///
/// Output port: `response` (string)
///
/// # Internal mode
///
/// Internal nodes declare a `prompt` input port and receive data from upstream nodes.
///
/// Input port: `prompt` (string)
/// Output port: `response` (string)
#[derive(Debug, Clone)]
pub struct LlmNode {
    /// System prompt to prepend to the user's prompt.
    system_prompt: String,
    /// Optional provider ID override.
    provider_id: Option<String>,
    /// If `Some`, this is a source node (zero input ports) that uses this prompt directly.
    /// If `None`, this is an internal node with a `prompt` input port.
    initial_prompt: Option<String>,
}

impl LlmNode {
    /// Create an internal LLM node with a `prompt` input port.
    ///
    /// The node receives its prompt from upstream nodes via the `prompt` input port.
    #[must_use]
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            provider_id: None,
            initial_prompt: None,
        }
    }

    /// Create a source LLM node with zero input ports.
    ///
    /// The `user_prompt` is embedded in the node and used directly as the user message.
    /// Source nodes are entry points in a workflow graph — they have no incoming edges.
    #[must_use]
    pub fn source(system_prompt: impl Into<String>, user_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            provider_id: None,
            initial_prompt: Some(user_prompt.into()),
        }
    }

    /// Set a specific provider ID for this node.
    #[must_use]
    pub fn with_provider(mut self, provider_id: impl Into<String>) -> Self {
        self.provider_id = Some(provider_id.into());
        self
    }
}

#[async_trait::async_trait]
impl WorkflowNode for LlmNode {
    fn name(&self) -> &'static str {
        "llm"
    }

    fn input_ports(&self) -> Vec<PortDef> {
        if self.initial_prompt.is_some() {
            vec![] // source node — no input ports
        } else {
            vec![PortDef::string("prompt")] // internal node
        }
    }

    fn output_ports(&self) -> Vec<PortDef> {
        vec![PortDef::string("response")]
    }

    async fn execute(
        &self,
        mut inputs: PortValues,
        ctx: &dyn NodeContext,
    ) -> Result<PortValues, Report<NodeError>> {
        let prompt = if let Some(ref initial) = self.initial_prompt {
            initial.clone()
        } else {
            inputs
                .take_string("prompt")
                .map_err(|e| Report::new(NodeError).attach(e.to_string()))?
        };

        let response = ctx
            .send_llm_request(&self.system_prompt, &prompt, self.provider_id.as_deref())
            .await?;

        let mut output = PortValues::new();
        output.insert("response".to_owned(), PortValue::String(response));
        Ok(output)
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(self.clone())
    }
}
