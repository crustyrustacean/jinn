use crate::{
    chat::{Tool, ToolChoice},
    error::LLMError,
    LLMProvider,
};

use crate::builder::build::helpers;
use crate::builder::state::BuilderState;

pub(super) fn build_lmstudio(
    state: &mut BuilderState,
    tools: Option<Vec<Tool>>,
    tool_choice: Option<ToolChoice>,
) -> Result<Box<dyn LLMProvider>, LLMError> {
    let api_key = helpers::optional_api_key(state).unwrap_or_else(|| "dummy-key".to_string());
    let timeout = helpers::timeout_or_default(state);
    let provider = crate::backends::lmstudio::LmStudio::with_config(
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
        None,
        None,
        state.reasoning_effort.take(),
        state.json_schema.take(),
        state.enable_parallel_tool_use,
        state.normalize_response,
    );
    Ok(Box::new(provider))
}
