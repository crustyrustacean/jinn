//! Tool calling domain: types, commands, and events for LLM tool use.
//!
//! Actors register tools via [`RegisterTools`], the LLM actor requests
//! execution via [`ExecuteToolBatch`], and the tool orchestrator coordinates
//! execution and emits results.

mod command;
mod event;
mod types;

pub use command::{ExecuteTool, ExecuteToolBatch, PushToolResult, RegisterTools};
pub use event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolUseStarted, ToolsRegistered,
};
pub use types::{ToolCall, ToolDefinition, ToolResult};
