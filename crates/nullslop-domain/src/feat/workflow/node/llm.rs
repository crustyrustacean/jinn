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
//!
//! # Validation & Self-Retry
//!
//! When configured with [`with_validation`](LlmNode::with_validation), the node
//! validates the LLM response against a regex pattern. If the response doesn't
//! match, the node retries up to `max_retries` times, appending a correction
//! prompt on each retry. All retrying happens inside `execute()` — the engine
//! sees a single node execution.

use error_stack::Report;
use nullslop_workflow::node::{NodeContext, NodeError, WorkflowNode};
use nullslop_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};
use nullslop_workflow::tool_schema::ToolSchema;
use regex::Regex;

/// A workflow node that calls the LLM.
///
/// # Configuration
///
/// - `system_prompt` — default system prompt. Overridden by the `system` input
///   port when connected.
/// - `provider_id` — optional provider ID. `None` uses the global default.
/// - `tool_schemas` — tool definitions available to the LLM during this call.
/// - `validation_regex` — optional regex for response validation.
/// - `max_retries` — maximum number of retries when validation fails (default 0).
/// - `retry_prompt` — correction prompt appended on retry.
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
    /// Regex pattern for validating the LLM response. `None` disables validation.
    validation_regex: Option<String>,
    /// Maximum number of retries when validation fails. 0 means no retries.
    max_retries: u32,
    /// Correction prompt appended on retry when validation fails.
    retry_prompt: Option<String>,
}

impl LlmNode {
    /// Create a new LLM node with the given default system prompt.
    #[must_use]
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: Some(system_prompt.into()),
            provider_id: None,
            tool_schemas: vec![],
            validation_regex: None,
            max_retries: 0,
            retry_prompt: None,
        }
    }

    /// Create an LLM node with no default system prompt.
    #[must_use]
    pub fn without_system_prompt() -> Self {
        Self {
            system_prompt: None,
            provider_id: None,
            tool_schemas: vec![],
            validation_regex: None,
            max_retries: 0,
            retry_prompt: None,
        }
    }

    /// Configure response validation with automatic retry.
    ///
    /// When set, the node validates each LLM response against `regex`.
    /// If the response doesn't match, the node retries up to `max_retries`
    /// times, appending `retry_prompt` as a correction on each retry.
    ///
    /// The total number of LLM calls is at most `max_retries + 1`.
    #[must_use]
    pub fn with_validation(
        mut self,
        regex: impl Into<String>,
        max_retries: u32,
        retry_prompt: impl Into<String>,
    ) -> Self {
        self.validation_regex = Some(regex.into());
        self.max_retries = max_retries;
        self.retry_prompt = Some(retry_prompt.into());
        self
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

        let original_message = match prompt_text {
            Some(prompt) => format!("{prompt}\n{user_text}"),
            None => user_text,
        };

        // No validation configured — single call, pass through.
        let Some(ref regex_pattern) = self.validation_regex else {
            let response = ctx
                .send_llm_request(
                    &original_message,
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
            return Ok(output);
        };

        // Validation configured — retry loop.
        let validation =
            Regex::new(regex_pattern).map_err(|_| Report::new(NodeError).attach("invalid validation regex"))?;
        let retry_prompt = self
            .retry_prompt
            .as_deref()
            .unwrap_or("Your response did not match the expected format. Please try again.");

        let mut current_prompt = original_message.clone();

        for attempt in 0..=self.max_retries {
            let response = ctx
                .send_llm_request(
                    &current_prompt,
                    system_prompt.as_deref(),
                    self.tool_schemas.clone(),
                    self.provider_id.as_deref(),
                )
                .await?;

            if validation.is_match(&response) {
                let mut output = PortValues::new();
                output.insert(
                    "response".to_owned(),
                    PortValue::single(ScalarValue::Text(response)),
                );
                return Ok(output);
            }

            // Validation failed — build retry prompt if we have attempts left.
            if attempt < self.max_retries {
                current_prompt = format!(
                    "{original_message}\n\n---\n\
                     Your previous response did not meet the expected format:\n\
                     \"{response}\"\n\n\
                     {retry_prompt}"
                );
            }
        }

        // All retries exhausted.
        Err(Report::new(NodeError).attach("LLM response validation failed after all retries"))
    }

    fn clone_box(&self) -> Box<dyn WorkflowNode> {
        Box::new(self.clone())
    }

    fn config(&self) -> Option<serde_json::Value> {
        let mut config = serde_json::json!({
            "system_prompt": self.system_prompt,
            "provider_id": self.provider_id,
        });

        if self.validation_regex.is_some() {
            config["validation_regex"] = serde_json::Value::String(
                self.validation_regex.clone().unwrap_or_default(),
            );
            config["max_retries"] = serde_json::Value::Number(self.max_retries.into());
            config["retry_prompt"] = serde_json::Value::String(
                self.retry_prompt.clone().unwrap_or_default(),
            );
        }

        Some(config)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;
    use std::sync::{Arc, Mutex};

    /// A `NodeContext` that returns predetermined responses in sequence.
    struct MockContext {
        responses: Arc<Mutex<Vec<String>>>,
        /// Records all prompts sent to send_llm_request.
        prompts: Arc<Mutex<Vec<String>>>,
    }

    impl MockContext {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
                prompts: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn prompt_count(&self) -> usize {
            self.prompts.lock().expect("lock").len()
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().expect("lock").clone()
        }
    }

    impl NodeContext for MockContext {
        fn send_llm_request<'a>(
            &'a self,
            user_prompt: &str,
            _system_prompt: Option<&str>,
            _tool_schemas: Vec<ToolSchema>,
            _provider_id: Option<&str>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, Report<NodeError>>> + Send + 'a>,
        > {
            let prompt = user_prompt.to_owned();
            let responses = self.responses.clone();
            let prompts = self.prompts.clone();
            Box::pin(async move {
                prompts.lock().expect("lock").push(prompt);
                let mut guard = responses.lock().expect("lock");
                guard
                    .pop()
                    .ok_or_else(|| Report::new(NodeError).attach("no more mock responses"))
            })
        }
    }

    fn make_inputs(user_text: &str) -> PortValues {
        let mut inputs = PortValues::new();
        inputs.insert(
            "user".to_owned(),
            PortValue::single(ScalarValue::Text(user_text.to_owned())),
        );
        inputs
    }

    #[tokio::test]
    async fn no_validation_passes_through_single_call() {
        // Given an LlmNode without validation.
        let node = LlmNode::new("You are helpful.");
        let ctx = MockContext::new(vec!["any response".to_owned()]);

        // When executing.
        let result = node.execute(make_inputs("hello"), &ctx).await;

        // Then the response passes through in a single call.
        let outputs = result.expect("should succeed");
        assert_eq!(outputs.get_text("response").unwrap(), "any response");
        assert_eq!(ctx.prompt_count(), 1);
    }

    #[tokio::test]
    async fn valid_response_passes_first_try() {
        // Given an LlmNode with validation that matches "YES".
        let node = LlmNode::new("You are a judge.")
            .with_validation(r"^YES$", 3, "Respond with YES or NO only.");
        let ctx = MockContext::new(vec!["YES".to_owned()]);

        // When executing.
        let result = node.execute(make_inputs("Is this valid?"), &ctx).await;

        // Then it succeeds on the first attempt.
        let outputs = result.expect("should succeed");
        assert_eq!(outputs.get_text("response").unwrap(), "YES");
        assert_eq!(ctx.prompt_count(), 1);
    }

    #[tokio::test]
    async fn invalid_response_triggers_retry() {
        // Given an LlmNode with validation.
        // Mock returns invalid first, then valid.
        let node = LlmNode::new("Judge.")
            .with_validation(r"^YES$", 2, "Respond with YES or NO only.");
        // Responses are popped from the end (stack), so last = first call.
        let ctx = MockContext::new(vec!["YES".to_owned(), "yes, but...".to_owned()]);

        // When executing.
        let result = node.execute(make_inputs("Check this."), &ctx).await;

        // Then it retries once and succeeds.
        let outputs = result.expect("should succeed");
        assert_eq!(outputs.get_text("response").unwrap(), "YES");
        assert_eq!(ctx.prompt_count(), 2);
    }

    #[tokio::test]
    async fn exhausts_all_retries_returns_error() {
        // Given an LlmNode with max_retries = 1 (2 total attempts).
        let node = LlmNode::new("Judge.")
            .with_validation(r"^YES$", 1, "Try again.");
        // Both responses are invalid.
        let ctx = MockContext::new(vec!["maybe".to_owned(), "I think so".to_owned()]);

        // When executing.
        let result = node.execute(make_inputs("Check this."), &ctx).await;

        // Then it returns an error after exhausting retries.
        assert!(result.is_err());
        assert_eq!(ctx.prompt_count(), 2);
    }

    #[tokio::test]
    async fn zero_max_retries_single_attempt() {
        // Given an LlmNode with max_retries = 0 (1 attempt only).
        let node = LlmNode::new("Judge.")
            .with_validation(r"^YES$", 0, "Try again.");
        let ctx = MockContext::new(vec!["nope".to_owned()]);

        // When executing.
        let result = node.execute(make_inputs("Check."), &ctx).await;

        // Then it fails after a single attempt (no retry).
        assert!(result.is_err());
        assert_eq!(ctx.prompt_count(), 1);
    }

    #[tokio::test]
    async fn retry_prompt_contains_original_and_correction() {
        // Given an LlmNode with validation.
        let node = LlmNode::new("Judge.")
            .with_validation(r"^YES$", 1, "Respond YES or NO.");
        // First invalid, second valid.
        let ctx = MockContext::new(vec!["YES".to_owned(), "maybe".to_owned()]);

        // When executing.
        let _ = node.execute(make_inputs("original question"), &ctx).await;

        // Then the second prompt contains the original question, the failed
        // response, and the retry correction.
        let prompts = ctx.prompts();
        assert_eq!(prompts.len(), 2);
        let retry = &prompts[1];
        assert!(
            retry.contains("original question"),
            "retry prompt must contain original: {retry}"
        );
        assert!(
            retry.contains("maybe"),
            "retry prompt must contain failed response: {retry}"
        );
        assert!(
            retry.contains("Respond YES or NO."),
            "retry prompt must contain correction: {retry}"
        );
    }

    #[tokio::test]
    async fn case_insensitive_regex_matches() {
        // Given an LlmNode with a case-insensitive regex.
        let node = LlmNode::new("Judge.")
            .with_validation(r"(?i)^yes$", 0, "Try again.");
        let ctx = MockContext::new(vec!["yes".to_owned()]);

        // When executing with lowercase "yes".
        let result = node.execute(make_inputs("Check."), &ctx).await;

        // Then it matches (case-insensitive) on the first attempt.
        let outputs = result.expect("should succeed");
        assert_eq!(outputs.get_text("response").unwrap(), "yes");
        assert_eq!(ctx.prompt_count(), 1);
    }

    #[test]
    fn config_includes_validation_when_configured() {
        // Given an LlmNode with validation.
        let node = LlmNode::new("Judge.")
            .with_validation(r"^YES$", 3, "Try again.");

        // When getting config.
        let config = node.config().expect("should have config");

        // Then it includes validation fields.
        assert_eq!(config["validation_regex"], serde_json::json!(r#"^YES$"#));
        assert_eq!(config["max_retries"], 3);
        assert_eq!(config["retry_prompt"], "Try again.");
    }

    #[test]
    fn config_excludes_validation_when_not_configured() {
        // Given an LlmNode without validation.
        let node = LlmNode::new("Helper.");

        // When getting config.
        let config = node.config().expect("should have config");

        // Then it does not include validation fields.
        assert!(config.get("validation_regex").is_none());
        assert!(config.get("max_retries").is_none());
    }
}
