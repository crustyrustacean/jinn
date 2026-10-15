//! Execution vocabulary shared by the orchestrator and every tool impl.

use std::future::Future;
use std::pin::Pin;

use jinn_core_types::tool_types::ToolResult;

/// The boxed future every built-in tool returns from its execute function.
pub type BoxedToolFuture = Pin<Box<dyn Future<Output = ToolResult> + Send>>;

/// The unique name of the `spawn_subagent` subagent tool.
pub const SPAWN_SUBAGENT_TOOL_NAME: &str = "spawn_subagent";
