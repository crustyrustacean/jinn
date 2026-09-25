//! Picker kind — identifies which picker is currently active
//! (shared vocabulary; this is the canonical home — the kernel re-exports it via `jinn_domain::protocol`).

use serde::{Deserialize, Serialize};

/// Which picker is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickerKind {
    /// Provider/model picker.
    Provider,
    /// Session browser picker.
    Session,
    /// Persona picker.
    Persona,
    /// Theme picker.
    Theme,
    /// Session lifecycle picker - select a lifecycle recipe for new session creation.
    SessionLifecycle,
    /// Reasoning effort picker - select reasoning effort for reasoning-capable models.
    ReasoningEffort,
    /// Tool picker - toggle which tools are enabled for the session.
    Tool,
    /// Skill picker - toggle which skills are enabled for the session.
    Skill,
    /// Task list browser - read-only zoom view of the active session's task list.
    TaskList,
    /// Project picker - curated project directories; create a new session rooted
    /// at the highlighted dir with `<enter>` (or `<c-enter>` to also pick a lifecycle).
    Project,
    /// MCP server picker - toggle which MCP servers are enabled for the session.
    McpServer,
    /// OpenRouter endpoint picker - pin a specific routing upstream for
    /// prefix-cache affinity on an OpenRouter-served Single model.
    Endpoint,
}

impl std::fmt::Display for PickerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider => write!(f, "models"),
            Self::Session => write!(f, "sessions"),
            Self::Persona => write!(f, "personas"),
            Self::Theme => write!(f, "themes"),

            Self::SessionLifecycle => write!(f, "session-lifecycle"),

            Self::ReasoningEffort => write!(f, "reasoning effort"),

            Self::Tool => write!(f, "tools"),
            Self::Skill => write!(f, "skills"),
            Self::TaskList => write!(f, "task list"),
            Self::Project => write!(f, "projects"),
            Self::McpServer => write!(f, "mcp servers"),

            Self::Endpoint => write!(f, "endpoints"),
        }
    }
}
