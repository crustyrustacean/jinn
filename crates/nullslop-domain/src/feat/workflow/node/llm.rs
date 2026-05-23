//! LLM workflow node.
//!
//! [`LlmNode`] is a workflow node that sends a prompt to the LLM and returns
//! the response as a string output port.

use error_stack::Report;
use nullslop_workflow::node::{NodeContext, NodeError, WorkflowNode};
use nullslop_workflow::port::{PortDef, PortValue, PortValues};

/// A workflow node that calls the LLM.
///
/// Input port: `prompt` (string)
/// Output port: `response` (string)
#[derive(Debug, Clone)]
pub struct LlmNode {
    /// System prompt to prepend to the user's prompt.
    system_prompt: String,
    /// Optional provider ID override.
    provider_id: Option<String>,
}

impl LlmNode {
    /// Create a new LLM node with the given system prompt.
    #[must_use]
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            provider_id: None,
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
        vec![PortDef::string("prompt")]
    }

    fn output_ports(&self) -> Vec<PortDef> {
        vec![PortDef::string("response")]
    }

    async fn execute(
        &self,
        mut inputs: PortValues,
        ctx: &dyn NodeContext,
    ) -> Result<PortValues, Report<NodeError>> {
        let prompt = inputs
            .take_string("prompt")
            .map_err(|e| Report::new(NodeError).attach(e.to_string()))?;

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
