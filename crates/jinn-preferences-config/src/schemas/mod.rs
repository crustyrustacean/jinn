//! Embedded configuration schemas for `jinn.toml` sections owned by other
//! features (prune, compaction, retry, lifecycles, projects, minimap, cwd
//! selector).
//!
//! Each submodule holds the pure serde *shape* of one config section plus
//! its `Configurable` impl, which is the whole registration: the key it
//! owns and, for a list, the field identifying an entry. The behavior
//! that consumes a section stays with its feature (kernel workers/actors
//! or slices).

pub mod auto_prune;
pub mod chat_log;
pub mod compaction;
pub mod cwd_selector;
pub mod minimap;
pub mod project;
pub mod provider;
pub mod request_retry;
pub mod session_lifecycle;
pub mod skills;
pub mod stall_watchdog;
pub mod tool_call_watchdog;
pub mod tools;

pub use auto_prune::{
    AutoPruneConfig, BrokenEditAutoPruneConfig, ConsecutiveReadsAutoPruneConfig,
    DoubleEditAutoPruneConfig, EditReadAutoPruneConfig, ReadEditAutoPruneConfig,
    RegexAutoPruneConfig, RegexPruneRule, ToolAgeWindowAutoPruneConfig,
    TrivialAssistantAutoPruneConfig,
};
pub use chat_log::ChatLogConfig;
pub use compaction::CompactionConfig;
pub use cwd_selector::CwdSelectorConfig;
pub use minimap::MinimapConfig;
pub use project::ProjectConfig;
pub use provider::WebSearchConfig;
pub use request_retry::RequestRetryConfig;
pub use session_lifecycle::{BuiltinId, LifecycleCommand, SessionLifecycle};
pub use skills::SkillsConfig;
pub use stall_watchdog::StallWatchdogConfig;
pub use tool_call_watchdog::ToolCallWatchdogConfig;
pub use tools::ToolsConfig;
