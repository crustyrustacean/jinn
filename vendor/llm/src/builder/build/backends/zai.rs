use crate::{error::LLMError, LLMProvider};

use super::super::helpers;
use crate::builder::state::BuilderState;

#[cfg(feature = "zai")]
pub(super) fn build_zai(state: &mut BuilderState) -> Result<Box<dyn LLMProvider>, LLMError> {
    let api_key = helpers::require_api_key(state, "ZAI")?;
    let timeout = helpers::timeout_or_default(state);

    let tools = state.tools.take();
    let tool_choice = state.tool_choice.take();

    let provider = crate::backends::zai::Zai::with_config(
        api_key,
        state.base_url.take(),
        state.model.take(),
        state.max_tokens,
        state.temperature,
        timeout,
        state.system.take(),
        state.top_p,
        state.top_k,
        tools,
        tool_choice,
        state.extra_body.take(),
        state.embedding_encoding_format.take(),
        state.embedding_dimensions,
        state.reasoning_effort.take(),
        state.json_schema.take(),
        state.enable_parallel_tool_use,
        state.normalize_response,
    );

    Ok(Box::new(provider))
}

#[cfg(not(feature = "zai"))]
pub(super) fn build_zai(_state: &mut BuilderState) -> Result<Box<dyn LLMProvider>, LLMError> {
    Err(LLMError::InvalidRequest(
        "ZAI feature not enabled".to_string(),
    ))
}
