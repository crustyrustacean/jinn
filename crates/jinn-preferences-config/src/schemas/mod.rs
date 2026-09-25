//! Embedded configuration schemas for `jinn.toml` sections owned by other
//! features (prune, compaction, retry, lifecycles, projects, minimap, cwd
//! selector).
//!
//! Each submodule holds the pure serde *shape* of one config section; the
//! behavior that consumes it stays with its feature (kernel workers/actors
//! or slices). The [`UserPreferences`](crate::UserPreferences) aggregate
//! re-exports the section types at the crate root.

pub mod auto_prune;
pub mod compaction;
pub mod cwd_selector;
pub mod minimap;
pub mod project;
pub mod request_retry;
pub mod session_lifecycle;
pub mod stall_watchdog;
pub mod tool_call_watchdog;

pub use auto_prune::{
    AutoPruneConfig, BrokenEditAutoPruneConfig, ConsecutiveReadsAutoPruneConfig,
    DoubleEditAutoPruneConfig, EditReadAutoPruneConfig, ReadEditAutoPruneConfig,
    RegexAutoPruneConfig, RegexPruneRule, ToolAgeWindowAutoPruneConfig,
    TrivialAssistantAutoPruneConfig,
};
pub use compaction::CompactionConfig;
pub use cwd_selector::CwdSelectorConfig;
pub use minimap::MinimapConfig;
pub use project::ProjectConfig;
pub use request_retry::RequestRetryConfig;
pub use session_lifecycle::{BuiltinId, LifecycleCommand, SessionLifecycle};
pub use stall_watchdog::StallWatchdogConfig;
pub use tool_call_watchdog::ToolCallWatchdogConfig;
