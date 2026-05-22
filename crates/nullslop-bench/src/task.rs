//! Bench task definitions — what to run and how to verify results.

#![allow(dead_code, reason = "task types are used by bench_actor and bench_tasks modules")]

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use nullslop_domain::feat::tools_actor::tool_types::ToolContext;
use nullslop_provider::ToolDefinition;

/// A boxed future returned by tool execute functions.
pub type BoxedToolFuture = Pin<Box<dyn Future<Output = nullslop_provider::ToolResult> + Send>>;

/// A single benchmark task definition.
pub struct BenchTask {
    /// Human-readable task name (used in CSV, fixture paths, progress output).
    pub name: &'static str,
    /// Messages to send sequentially. Each message waits for `SessionPhase::Idle`
    /// before sending the next.
    pub messages: Vec<&'static str>,
    /// Fixture directory relative to `crates/nullslop-bench/fixtures/`.
    /// `None` means an empty working directory.
    pub fixture_dir: Option<&'static str>,
    /// Per-task timeout (total wall time for all messages).
    pub timeout: Duration,
    /// Persona name to activate before running. `None` = default persona.
    pub persona: Option<&'static str>,
    /// Which tools to make available for this task.
    pub tools: BenchTools,
    /// Verification function run against the fixture directory after completion.
    /// Returns `true` if the task succeeded.
    pub verify: fn(&Path) -> bool,
}

/// Tool configuration for a bench task.
pub struct BenchTools {
    /// Subset of built-in tool names to register (e.g., `["bash", "read", "write"]`).
    /// Empty means all built-in tools are registered.
    pub builtins: Vec<&'static str>,
    /// Additional custom tools with their definitions and execute functions.
    pub custom: Vec<CustomTool>,
}

/// A custom tool provided by a bench task.
#[derive(Clone)]
pub struct CustomTool {
    /// The tool's JSON-schema definition.
    pub definition: ToolDefinition,
    /// The function that executes the tool call.
    pub execute: fn(nullslop_provider::ToolCall, ToolContext) -> BoxedToolFuture,
}
