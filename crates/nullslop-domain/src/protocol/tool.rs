//! Tool calling domain: types, commands, and events for LLM tool use.

pub use crate::feat::tools::protocol::command::{
    ExecuteTool, ExecuteToolBatch, PushToolResult, RegisterTools,
};
pub use crate::feat::tools::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolUseStarted, ToolsRegistered,
};
pub use crate::feat::tools::tool_types::{ToolCall, ToolDefinition, ToolResult};
