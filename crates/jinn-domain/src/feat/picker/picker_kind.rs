//! Picker kind - identifies which picker is currently active.
//!
//! A single set of `Picker*` commands, `Mode::Picker`, per-picker `Scope`,
//! and keymap bindings serve all pickers. [`PickerKind`] determines which
//! `SelectionState` (from `jinn-selection-widget`) the commands
//! operate on.

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
    /// Plugin picker - select a plugin to attach to the session.
    Plugin,

    /// Compaction model picker - select a model for context compaction summarization.
    CompactionModel,
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
}

impl std::fmt::Display for PickerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider => write!(f, "models"),
            Self::Plugin => write!(f, "plugins"),
            Self::Session => write!(f, "sessions"),
            Self::Persona => write!(f, "personas"),
            Self::Theme => write!(f, "themes"),

            Self::SessionLifecycle => write!(f, "session-lifecycle"),

            Self::CompactionModel => write!(f, "compaction model"),

            Self::ReasoningEffort => write!(f, "reasoning effort"),

            Self::Tool => write!(f, "tools"),
            Self::Skill => write!(f, "skills"),
            Self::TaskList => write!(f, "task list"),
            Self::Project => write!(f, "projects"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;

    #[test]
    fn reasoning_effort_displays_as_reasoning_effort() {
        // Given the ReasoningEffort picker kind.
        // When displaying.
        // Then it renders as 'reasoning effort'.
        assert_eq!(PickerKind::ReasoningEffort.to_string(), "reasoning effort");
    }
}
